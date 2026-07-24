use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{ObjectMeta, ObjectStorage, StorageError};

/// In-memory storage backend for unit and integration tests.
///
/// Thread-safe via `Arc<RwLock<HashMap>>`. All data lives in process memory
/// and is lost when the instance is dropped — no disk, no network, no config.
///
/// ```rust,ignore
/// use platform_storage::memory::InMemoryStorage;
///
/// let storage = InMemoryStorage::new();
/// storage.put("key", Bytes::from("data"), "text/plain").await?;
/// let data = storage.get("key").await?;
/// ```
#[derive(Clone, Default)]
pub struct InMemoryStorage {
    objects: Arc<RwLock<HashMap<String, StoredObject>>>,
}

#[derive(Clone)]
struct StoredObject {
    data: Bytes,
    content_type: String,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the number of objects currently stored.
    pub async fn len(&self) -> usize {
        self.objects.read().await.len()
    }

    /// Check if the store is empty.
    pub async fn is_empty(&self) -> bool {
        self.objects.read().await.is_empty()
    }

    /// List all keys in the store (useful for test assertions).
    pub async fn keys(&self) -> Vec<String> {
        self.objects.read().await.keys().cloned().collect()
    }

    /// Clear all objects (reset between tests).
    pub async fn clear(&self) {
        self.objects.write().await.clear();
    }
}

impl ObjectStorage for InMemoryStorage {
    async fn put(&self, key: &str, data: Bytes, content_type: &str) -> Result<(), StorageError> {
        self.objects.write().await.insert(
            key.to_string(),
            StoredObject {
                data,
                content_type: content_type.to_string(),
            },
        );
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
        let mut buf = BytesMut::new();
        while let Some(item) = stream.next().await {
            buf.extend_from_slice(&item?);
        }
        let total = buf.len() as u64;
        self.put(key, buf.freeze(), content_type).await?;
        Ok(total)
    }

    async fn get(&self, key: &str) -> Result<Bytes, StorageError> {
        self.objects
            .read()
            .await
            .get(key)
            .map(|obj| obj.data.clone())
            .ok_or_else(|| StorageError::NotFound {
                key: key.to_string(),
            })
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        Ok(self.objects.read().await.contains_key(key))
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.objects.write().await.remove(key);
        Ok(())
    }

    async fn delete_prefix(&self, prefix: &str) -> Result<usize, StorageError> {
        let mut objects = self.objects.write().await;
        let matching: Vec<String> = objects
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        for key in &matching {
            objects.remove(key);
        }
        Ok(matching.len())
    }

    async fn list_prefix(&self, prefix: &str) -> Result<Vec<ObjectMeta>, StorageError> {
        let objects = self.objects.read().await;
        let mut matching: Vec<ObjectMeta> = objects
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(key, object)| ObjectMeta {
                key: key.clone(),
                size: object.data.len() as i64,
                content_type: Some(object.content_type.clone()),
            })
            .collect();
        matching.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(matching)
    }

    async fn presigned_url(&self, key: &str, _expiry_secs: u64) -> Result<String, StorageError> {
        // Verify object exists, then return a fake URL for test assertions
        if !self.objects.read().await.contains_key(key) {
            return Err(StorageError::NotFound {
                key: key.to_string(),
            });
        }
        Ok(format!("memory://{key}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_and_get() {
        let store = InMemoryStorage::new();
        store
            .put("test/file.txt", Bytes::from("hello"), "text/plain")
            .await
            .unwrap();

        let data = store.get("test/file.txt").await.unwrap();
        assert_eq!(data, Bytes::from("hello"));
    }

    #[tokio::test]
    async fn get_missing_key_returns_not_found() {
        let store = InMemoryStorage::new();
        let result = store.get("nonexistent").await;
        assert!(matches!(result, Err(StorageError::NotFound { .. })));
    }

    #[tokio::test]
    async fn exists_returns_true_for_stored_key() {
        let store = InMemoryStorage::new();
        store
            .put("key", Bytes::from("data"), "application/octet-stream")
            .await
            .unwrap();

        assert!(store.exists("key").await.unwrap());
        assert!(!store.exists("other").await.unwrap());
    }

    #[tokio::test]
    async fn delete_removes_object() {
        let store = InMemoryStorage::new();
        store
            .put("key", Bytes::from("data"), "text/plain")
            .await
            .unwrap();

        store.delete("key").await.unwrap();
        assert!(!store.exists("key").await.unwrap());
    }

    #[tokio::test]
    async fn delete_prefix_removes_matching_leaves_others() {
        let store = InMemoryStorage::new();
        for key in ["uploads/t1/a.pdf", "uploads/t1/b.pdf", "uploads/t2/c.pdf"] {
            store
                .put(key, Bytes::from("data"), "application/pdf")
                .await
                .unwrap();
        }

        let deleted = store.delete_prefix("uploads/t1/").await.unwrap();
        assert_eq!(deleted, 2);
        assert!(!store.exists("uploads/t1/a.pdf").await.unwrap());
        assert!(!store.exists("uploads/t1/b.pdf").await.unwrap());
        // A different tenant's objects are untouched.
        assert!(store.exists("uploads/t2/c.pdf").await.unwrap());
    }

    #[tokio::test]
    async fn delete_prefix_empty_is_ok_zero() {
        let store = InMemoryStorage::new();
        store
            .put("uploads/t1/a.pdf", Bytes::from("data"), "application/pdf")
            .await
            .unwrap();

        let deleted = store.delete_prefix("uploads/nonexistent/").await.unwrap();
        assert_eq!(deleted, 0);
        // Non-matching object still present.
        assert!(store.exists("uploads/t1/a.pdf").await.unwrap());
    }

    #[tokio::test]
    async fn list_prefix_returns_only_matching_keys_with_sizes() {
        let store = InMemoryStorage::new();
        store
            .put(
                "adapters/t1/m1/config.json",
                Bytes::from("{}"),
                "application/json",
            )
            .await
            .unwrap();
        store
            .put(
                "adapters/t1/m1/model.safetensors",
                Bytes::from("weights"),
                "application/octet-stream",
            )
            .await
            .unwrap();
        store
            .put(
                "adapters/t2/m9/config.json",
                Bytes::from("{}"),
                "application/json",
            )
            .await
            .unwrap();

        let listed = store.list_prefix("adapters/t1/m1/").await.unwrap();

        let keys: Vec<&str> = listed.iter().map(|o| o.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "adapters/t1/m1/config.json",
                "adapters/t1/m1/model.safetensors"
            ]
        );
        assert_eq!(listed[1].size, 7);
    }

    #[tokio::test]
    async fn list_prefix_with_no_matches_is_empty() {
        let store = InMemoryStorage::new();
        store
            .put("adapters/t1/a", Bytes::from("x"), "text/plain")
            .await
            .unwrap();

        assert!(
            store
                .list_prefix("adapters/none/")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn delete_nonexistent_is_noop() {
        let store = InMemoryStorage::new();
        // Should not error
        store.delete("nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn presigned_url_returns_memory_scheme() {
        let store = InMemoryStorage::new();
        store
            .put("docs/file.pdf", Bytes::from("pdf"), "application/pdf")
            .await
            .unwrap();

        let url = store.presigned_url("docs/file.pdf", 3600).await.unwrap();
        assert_eq!(url, "memory://docs/file.pdf");
    }

    #[tokio::test]
    async fn presigned_url_missing_key_returns_not_found() {
        let store = InMemoryStorage::new();
        let result = store.presigned_url("missing", 3600).await;
        assert!(matches!(result, Err(StorageError::NotFound { .. })));
    }

    #[tokio::test]
    async fn overwrite_replaces_data() {
        let store = InMemoryStorage::new();
        store
            .put("key", Bytes::from("v1"), "text/plain")
            .await
            .unwrap();
        store
            .put("key", Bytes::from("v2"), "text/plain")
            .await
            .unwrap();

        let data = store.get("key").await.unwrap();
        assert_eq!(data, Bytes::from("v2"));
        assert_eq!(store.len().await, 1);
    }

    #[tokio::test]
    async fn clear_removes_all() {
        let store = InMemoryStorage::new();
        store
            .put("a", Bytes::from("1"), "text/plain")
            .await
            .unwrap();
        store
            .put("b", Bytes::from("2"), "text/plain")
            .await
            .unwrap();

        assert_eq!(store.len().await, 2);
        store.clear().await;
        assert!(store.is_empty().await);
    }

    #[tokio::test]
    async fn put_streaming_concatenates_chunks() {
        let store = InMemoryStorage::new();
        let chunks: Vec<Result<Bytes, StorageError>> =
            vec![Ok(Bytes::from("hel")), Ok(Bytes::from("lo "))];
        let total = store
            .put_streaming("k", futures::stream::iter(chunks), "text/plain")
            .await
            .unwrap();
        assert_eq!(total, 6);
        assert_eq!(store.get("k").await.unwrap(), Bytes::from("hello "));
    }

    #[tokio::test]
    async fn put_streaming_empty_stream_writes_zero_bytes() {
        let store = InMemoryStorage::new();
        let empty: Vec<Result<Bytes, StorageError>> = vec![];
        let total = store
            .put_streaming("k", futures::stream::iter(empty), "text/plain")
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert_eq!(store.get("k").await.unwrap(), Bytes::new());
    }

    #[tokio::test]
    async fn put_streaming_propagates_stream_error() {
        let store = InMemoryStorage::new();
        let chunks: Vec<Result<Bytes, StorageError>> = vec![
            Ok(Bytes::from("partial")),
            Err(StorageError::UploadFailed("aborted".into())),
        ];
        let result = store
            .put_streaming("k", futures::stream::iter(chunks), "text/plain")
            .await;
        assert!(matches!(result, Err(StorageError::UploadFailed(_))));
        // Nothing committed on abort.
        assert!(!store.exists("k").await.unwrap());
    }

    #[tokio::test]
    async fn clone_shares_state() {
        let store = InMemoryStorage::new();
        let clone = store.clone();

        store
            .put("key", Bytes::from("data"), "text/plain")
            .await
            .unwrap();

        // Clone sees the same data (Arc<RwLock> shared)
        let data = clone.get("key").await.unwrap();
        assert_eq!(data, Bytes::from("data"));
    }
}
