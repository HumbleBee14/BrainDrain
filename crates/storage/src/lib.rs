pub mod memory;
pub mod s3;

use bytes::Bytes;
use futures::Stream;
use std::future::Future;

/// Errors from storage operations.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Object not found: {key}")]
    NotFound { key: String },

    #[error("Upload failed: {0}")]
    UploadFailed(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Delete failed: {0}")]
    DeleteFailed(String),

    #[error("Presign failed: {0}")]
    PresignFailed(String),

    #[error("Storage backend error: {0}")]
    Backend(String),
}

/// Metadata about a stored object.
#[derive(Debug, Clone)]
pub struct ObjectMeta {
    pub key: String,
    pub size: i64,
    pub content_type: Option<String>,
}

/// Storage abstraction trait.
///
/// Decouples the application from any specific storage backend (S3, R2, GCS, local, in-memory).
/// All methods return boxed futures to allow object-safe dynamic dispatch.
pub trait ObjectStorage: Send + Sync {
    /// Upload bytes to the given key.
    fn put(
        &self,
        key: &str,
        data: Bytes,
        content_type: &str,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Stream bytes to the given key without buffering the whole object in
    /// memory, returning the total number of bytes written. Backends that
    /// support it use multipart upload so a large file never lands in RAM in
    /// full. The stream may yield `Err` to abort the upload (e.g. an upstream
    /// size cap tripped); the partial object is then discarded, not committed.
    fn put_streaming<S>(
        &self,
        key: &str,
        stream: S,
        content_type: &str,
    ) -> impl Future<Output = Result<u64, StorageError>> + Send
    where
        S: Stream<Item = Result<Bytes, StorageError>> + Send + 'static;

    /// Download object as bytes.
    fn get(&self, key: &str) -> impl Future<Output = Result<Bytes, StorageError>> + Send;

    /// Check if an object exists.
    fn exists(&self, key: &str) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Delete an object.
    fn delete(&self, key: &str) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Delete every object whose key starts with `prefix`, returning the count
    /// deleted. An empty prefix (no matching objects) is `Ok(0)`. Deleting the
    /// same prefix twice is safe: the second call finds nothing and returns 0.
    fn delete_prefix(
        &self,
        prefix: &str,
    ) -> impl Future<Output = Result<usize, StorageError>> + Send;

    /// List every object whose key starts with `prefix`.
    fn list_prefix(
        &self,
        prefix: &str,
    ) -> impl Future<Output = Result<Vec<ObjectMeta>, StorageError>> + Send;

    /// Generate a presigned download URL valid for `expiry_secs`.
    fn presigned_url(
        &self,
        key: &str,
        expiry_secs: u64,
    ) -> impl Future<Output = Result<String, StorageError>> + Send;
}
