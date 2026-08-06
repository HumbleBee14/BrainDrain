use uuid::Uuid;

use platform_shared::s3_paths;
use platform_storage::ObjectStorage;

use crate::dto::admin::TenantErasureSummary;
use crate::error::{AppError, AppResult};
use crate::repositories::traits::TenantRepository;

// StorageError converts to AppError::Storage via `?` (From impl in error.rs).

/// Platform-admin tenant erasure (GDPR right-to-erasure / offboarding).
pub struct TenantErasureService;

impl TenantErasureService {
    /// Wipe all of a tenant's personal/operational data and every S3 object,
    /// while retaining billing and audit records (they carry no tenant FK).
    ///
    /// Ordering is deliberate: S3 objects are deleted FIRST, then the tenant
    /// row (which cascades operational tables). If any prefix delete fails we
    /// abort before touching the DB and return the error, leaving the tenant
    /// row intact so the operation is retryable — re-running re-deletes any
    /// remaining objects (deleting already-gone keys is a no-op) and then
    /// proceeds. This guarantees we never drop the records proving the tenant
    /// existed while its PII objects still linger in storage.
    pub async fn erase_tenant(
        tenant_repo: &dyn TenantRepository,
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
    ) -> AppResult<TenantErasureSummary> {
        let mut objects_deleted: usize = 0;
        for prefix in s3_paths::tenant_prefixes(tenant_id) {
            objects_deleted += storage.delete_prefix(&prefix).await?;
        }

        let deleted = tenant_repo.delete(tenant_id).await?;
        if !deleted {
            return Err(AppError::NotFound {
                message: "Tenant not found".to_string(),
            });
        }

        tracing::info!(
            tenant_id = %tenant_id,
            objects_deleted = objects_deleted,
            "Tenant erased"
        );

        Ok(TenantErasureSummary {
            tenant_id: tenant_id.to_string(),
            objects_deleted: objects_deleted as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::traits::BoxFuture;
    use bytes::Bytes;
    use futures::Stream;
    use platform_db::models::Tenant;
    use platform_storage::StorageError;
    use platform_storage::memory::InMemoryStorage;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Fake tenant repo: records whether `delete` was called and what it returns.
    struct FakeTenantRepo {
        delete_called: Arc<AtomicBool>,
        delete_returns: bool,
    }

    impl FakeTenantRepo {
        fn new(delete_returns: bool) -> Self {
            Self {
                delete_called: Arc::new(AtomicBool::new(false)),
                delete_returns,
            }
        }
    }

    impl TenantRepository for FakeTenantRepo {
        fn get_by_id(&self, _id: Uuid) -> BoxFuture<'_, AppResult<Option<Tenant>>> {
            unimplemented!()
        }
        fn sum_storage_bytes(&self, _tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
            unimplemented!("erasure never reads storage usage")
        }
        fn update_stripe_customer(
            &self,
            _id: Uuid,
            _customer_id: &str,
        ) -> BoxFuture<'_, AppResult<()>> {
            unimplemented!()
        }
        fn update_subscription(
            &self,
            _id: Uuid,
            _subscription_id: &str,
            _plan: &str,
            _limits: serde_json::Value,
        ) -> BoxFuture<'_, AppResult<()>> {
            unimplemented!()
        }
        fn get_by_stripe_customer(
            &self,
            _customer_id: &str,
        ) -> BoxFuture<'_, AppResult<Option<Tenant>>> {
            unimplemented!()
        }
        fn get_plan_limits(&self, _id: Uuid) -> BoxFuture<'_, AppResult<serde_json::Value>> {
            unimplemented!()
        }
        fn get_settings(&self, _id: Uuid) -> BoxFuture<'_, AppResult<serde_json::Value>> {
            unimplemented!()
        }
        fn update_settings(
            &self,
            _id: Uuid,
            _settings: serde_json::Value,
        ) -> BoxFuture<'_, AppResult<()>> {
            unimplemented!()
        }
        fn delete(&self, _id: Uuid) -> BoxFuture<'_, AppResult<bool>> {
            self.delete_called.store(true, Ordering::SeqCst);
            let returns = self.delete_returns;
            Box::pin(async move { Ok(returns) })
        }
    }

    /// Storage whose `delete_prefix` always errors; other methods delegate to
    /// an in-memory backend (unused here).
    #[derive(Default)]
    struct FailingPrefixStorage {
        inner: InMemoryStorage,
    }

    impl ObjectStorage for FailingPrefixStorage {
        fn put(
            &self,
            key: &str,
            data: Bytes,
            content_type: &str,
        ) -> impl std::future::Future<Output = Result<(), StorageError>> + Send {
            self.inner.put(key, data, content_type)
        }
        fn put_streaming<S>(
            &self,
            key: &str,
            stream: S,
            content_type: &str,
        ) -> impl std::future::Future<Output = Result<u64, StorageError>> + Send
        where
            S: Stream<Item = Result<Bytes, StorageError>> + Send + 'static,
        {
            self.inner.put_streaming(key, stream, content_type)
        }
        fn get(
            &self,
            key: &str,
        ) -> impl std::future::Future<Output = Result<Bytes, StorageError>> + Send {
            self.inner.get(key)
        }
        fn exists(
            &self,
            key: &str,
        ) -> impl std::future::Future<Output = Result<bool, StorageError>> + Send {
            self.inner.exists(key)
        }
        fn delete(
            &self,
            key: &str,
        ) -> impl std::future::Future<Output = Result<(), StorageError>> + Send {
            self.inner.delete(key)
        }
        async fn delete_prefix(&self, _prefix: &str) -> Result<usize, StorageError> {
            Err(StorageError::DeleteFailed("boom".to_string()))
        }
        fn list_prefix(
            &self,
            prefix: &str,
        ) -> impl std::future::Future<
            Output = Result<Vec<platform_storage::ObjectMeta>, StorageError>,
        > + Send {
            self.inner.list_prefix(prefix)
        }
        fn presigned_url(
            &self,
            key: &str,
            expiry_secs: u64,
        ) -> impl std::future::Future<Output = Result<String, StorageError>> + Send {
            self.inner.presigned_url(key, expiry_secs)
        }
    }

    #[tokio::test]
    async fn erase_deletes_all_prefixes_and_row() {
        let tenant_id = Uuid::new_v4();
        let storage = InMemoryStorage::new();
        // Seed one object under every tenant prefix plus an unrelated one.
        for prefix in s3_paths::tenant_prefixes(tenant_id) {
            storage
                .put(&format!("{prefix}obj"), Bytes::from("x"), "text/plain")
                .await
                .unwrap();
        }
        storage
            .put("uploads/other-tenant/keep", Bytes::from("x"), "text/plain")
            .await
            .unwrap();

        let repo = FakeTenantRepo::new(true);
        let called = repo.delete_called.clone();

        let summary = TenantErasureService::erase_tenant(&repo, &storage, tenant_id)
            .await
            .unwrap();

        assert_eq!(summary.objects_deleted, 9);
        assert_eq!(summary.tenant_id, tenant_id.to_string());
        assert!(called.load(Ordering::SeqCst), "repo.delete must be called");
        // Only the other tenant's object survives.
        assert_eq!(storage.len().await, 1);
        assert!(storage.exists("uploads/other-tenant/keep").await.unwrap());
    }

    #[tokio::test]
    async fn prefix_delete_error_skips_db_delete() {
        let tenant_id = Uuid::new_v4();
        let storage = FailingPrefixStorage::default();
        let repo = FakeTenantRepo::new(true);
        let called = repo.delete_called.clone();

        let result = TenantErasureService::erase_tenant(&repo, &storage, tenant_id).await;

        assert!(result.is_err(), "must propagate the storage error");
        assert!(
            !called.load(Ordering::SeqCst),
            "repo.delete must NOT be called when a prefix delete fails (retry-safety)",
        );
    }

    #[tokio::test]
    async fn missing_tenant_returns_not_found() {
        let tenant_id = Uuid::new_v4();
        let storage = InMemoryStorage::new();
        let repo = FakeTenantRepo::new(false);

        let result = TenantErasureService::erase_tenant(&repo, &storage, tenant_id).await;

        assert!(matches!(result, Err(AppError::NotFound { .. })));
    }
}
