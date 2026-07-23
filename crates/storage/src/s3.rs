use aws_sdk_s3::Client;
use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use std::time::Duration;

use crate::{ObjectStorage, StorageError};

/// Multipart part size. S3 requires every part except the last to be at least
/// 5 MiB; 8 MiB keeps part counts low for large objects while bounding the
/// in-flight buffer.
const MULTIPART_PART_SIZE: usize = 8 * 1024 * 1024;

/// S3-compatible storage backend.
///
/// Works with AWS S3, Cloudflare R2, MinIO, and any S3-compatible provider.
/// Configuration is injected at construction — no hardcoded endpoints.
#[derive(Clone)]
pub struct S3Storage {
    client: Client,
    bucket: String,
}

/// Configuration for creating an S3Storage instance.
pub struct S3Config {
    pub endpoint: Option<String>,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub bucket: String,
    /// Force path-style access (required for MinIO and some S3-compatible stores).
    pub force_path_style: bool,
}

impl S3Storage {
    /// Create a new S3Storage with the given configuration.
    pub async fn new(config: S3Config) -> Self {
        let credentials = Credentials::new(
            &config.access_key,
            &config.secret_key,
            None,
            None,
            "platform",
        );

        let mut s3_config_builder = aws_sdk_s3::Config::builder()
            // Required by aws-sdk-s3 1.x when building a Config directly (without
            // aws-config's loader): the client panics at construction otherwise.
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .credentials_provider(credentials)
            .region(Region::new(config.region))
            .force_path_style(config.force_path_style)
            // Only add checksums when the operation requires them. aws-sdk-s3 (like
            // botocore >=1.36) defaults to "when_supported", which attaches a CRC32
            // checksum to every PutObject — non-AWS S3 stores (Cloudflare R2, some
            // MinIO versions) reject that on plain uploads. "when_required" restores
            // the widely-compatible behavior; real AWS S3 still fully supports it.
            .request_checksum_calculation(
                aws_sdk_s3::config::RequestChecksumCalculation::WhenRequired,
            )
            .response_checksum_validation(
                aws_sdk_s3::config::ResponseChecksumValidation::WhenRequired,
            );

        if let Some(endpoint) = config.endpoint {
            s3_config_builder = s3_config_builder.endpoint_url(endpoint);
        }

        let client = Client::from_conf(s3_config_builder.build());

        Self {
            client,
            bucket: config.bucket,
        }
    }

    /// Upload one part of a multipart upload, returning its completed-part handle.
    async fn upload_part(
        &self,
        key: &str,
        upload_id: &str,
        part_number: i32,
        body: Bytes,
    ) -> Result<CompletedPart, StorageError> {
        let resp = self
            .client
            .upload_part()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(upload_id)
            .part_number(part_number)
            .body(ByteStream::from(body))
            .send()
            .await
            .map_err(|e| StorageError::UploadFailed(e.to_string()))?;

        Ok(CompletedPart::builder()
            .part_number(part_number)
            .set_e_tag(resp.e_tag().map(str::to_string))
            .build())
    }

    /// Abort a multipart upload, discarding any parts already uploaded. Errors
    /// are logged, not propagated — the caller is already returning an error.
    async fn abort_multipart(&self, key: &str, upload_id: &str) {
        if let Err(e) = self
            .client
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(upload_id)
            .send()
            .await
        {
            tracing::warn!(key = key, error = %e, "Failed to abort multipart upload");
        }
    }

    /// Ensure the bucket exists, creating it if necessary.
    /// Useful for local dev with MinIO.
    pub async fn ensure_bucket(&self) -> Result<(), StorageError> {
        match self.client.head_bucket().bucket(&self.bucket).send().await {
            Ok(_) => Ok(()),
            Err(_) => {
                self.client
                    .create_bucket()
                    .bucket(&self.bucket)
                    .send()
                    .await
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok(())
            }
        }
    }
}

impl ObjectStorage for S3Storage {
    async fn put(&self, key: &str, data: Bytes, content_type: &str) -> Result<(), StorageError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(data))
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| StorageError::UploadFailed(e.to_string()))?;

        tracing::debug!(key = key, "Object uploaded to S3");
        Ok(())
    }

    async fn put_streaming<S>(
        &self,
        key: &str,
        stream: S,
        content_type: &str,
    ) -> Result<u64, StorageError>
    where
        S: Stream<Item = Result<Bytes, StorageError>> + Send + 'static,
    {
        futures::pin_mut!(stream);

        let mut buf = BytesMut::with_capacity(MULTIPART_PART_SIZE);
        let mut total: u64 = 0;
        // Lazily promote to a multipart upload only once the object grows past a
        // single part. Small objects take the plain PutObject path, which also
        // handles the empty-object case that multipart cannot express.
        let mut upload_id: Option<String> = None;
        let mut parts: Vec<CompletedPart> = Vec::new();
        let mut part_number = 1;

        while let Some(item) = stream.next().await {
            let chunk = match item {
                Ok(c) => c,
                Err(e) => {
                    if let Some(uid) = &upload_id {
                        self.abort_multipart(key, uid).await;
                    }
                    return Err(e);
                }
            };
            total += chunk.len() as u64;
            buf.extend_from_slice(&chunk);

            while buf.len() >= MULTIPART_PART_SIZE {
                if upload_id.is_none() {
                    let created = self
                        .client
                        .create_multipart_upload()
                        .bucket(&self.bucket)
                        .key(key)
                        .content_type(content_type)
                        .send()
                        .await
                        .map_err(|e| StorageError::UploadFailed(e.to_string()))?;
                    upload_id = created.upload_id().map(str::to_string);
                    if upload_id.is_none() {
                        return Err(StorageError::UploadFailed(
                            "S3 did not return a multipart upload id".to_string(),
                        ));
                    }
                }
                let uid = upload_id.as_deref().expect("upload_id set above");
                let part = buf.split_to(MULTIPART_PART_SIZE).freeze();
                match self.upload_part(key, uid, part_number, part).await {
                    Ok(cp) => parts.push(cp),
                    Err(e) => {
                        self.abort_multipart(key, uid).await;
                        return Err(e);
                    }
                }
                part_number += 1;
            }
        }

        match upload_id {
            // Never grew past one part: a single PutObject with a now-known length.
            None => {
                self.put(key, buf.freeze(), content_type).await?;
            }
            Some(uid) => {
                if !buf.is_empty() {
                    match self.upload_part(key, &uid, part_number, buf.freeze()).await {
                        Ok(cp) => parts.push(cp),
                        Err(e) => {
                            self.abort_multipart(key, &uid).await;
                            return Err(e);
                        }
                    }
                }
                let completed = CompletedMultipartUpload::builder()
                    .set_parts(Some(parts))
                    .build();
                if let Err(e) = self
                    .client
                    .complete_multipart_upload()
                    .bucket(&self.bucket)
                    .key(key)
                    .upload_id(&uid)
                    .multipart_upload(completed)
                    .send()
                    .await
                {
                    self.abort_multipart(key, &uid).await;
                    return Err(StorageError::UploadFailed(e.to_string()));
                }
            }
        }

        tracing::debug!(key = key, bytes = total, "Object streamed to S3");
        Ok(total)
    }

    async fn get(&self, key: &str) -> Result<Bytes, StorageError> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("NoSuchKey") || msg.contains("404") {
                    StorageError::NotFound {
                        key: key.to_string(),
                    }
                } else {
                    StorageError::DownloadFailed(msg)
                }
            })?;

        let data = resp
            .body
            .collect()
            .await
            .map_err(|e| StorageError::DownloadFailed(e.to_string()))?
            .into_bytes();

        Ok(data)
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("404") || msg.contains("NotFound") || msg.contains("NoSuchKey") {
                    Ok(false)
                } else {
                    Err(StorageError::Backend(msg))
                }
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;

        tracing::debug!(key = key, "Object deleted from S3");
        Ok(())
    }

    async fn presigned_url(&self, key: &str, expiry_secs: u64) -> Result<String, StorageError> {
        let presigning = PresigningConfig::builder()
            .expires_in(Duration::from_secs(expiry_secs))
            .build()
            .map_err(|e| StorageError::PresignFailed(e.to_string()))?;

        let req = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presigning)
            .await
            .map_err(|e| StorageError::PresignFailed(e.to_string()))?;

        Ok(req.uri().to_string())
    }
}
