pub mod memory;
pub mod s3;

use bytes::Bytes;
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

    /// Download object as bytes.
    fn get(&self, key: &str) -> impl Future<Output = Result<Bytes, StorageError>> + Send;

    /// Check if an object exists.
    fn exists(&self, key: &str) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Delete an object.
    fn delete(&self, key: &str) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Generate a presigned download URL valid for `expiry_secs`.
    fn presigned_url(
        &self,
        key: &str,
        expiry_secs: u64,
    ) -> impl Future<Output = Result<String, StorageError>> + Send;
}
