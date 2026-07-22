use platform_db::models::ApiKey;
use platform_db::tenant::begin_tenant_tx;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{ApiKeyRepository, BoxFuture};

/// PostgreSQL implementation of the API key repository.
///
/// Tenant-scoped management queries run on the RLS pool (`db`) inside a
/// tenant-scoped transaction. Auth-by-hash lookups happen before a tenant is
/// known, so they run on the owner pool (`db_admin`), which is exempt from RLS.
pub struct PgApiKeyRepo {
    db: PgPool,
    db_admin: PgPool,
}

impl PgApiKeyRepo {
    pub fn new(db: PgPool, db_admin: PgPool) -> Self {
        Self { db, db_admin }
    }
}

impl ApiKeyRepository for PgApiKeyRepo {
    #[allow(clippy::too_many_arguments)]
    fn create(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        name: &str,
        key_prefix: &str,
        key_hash: &str,
        rate_limit: i32,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> BoxFuture<'_, AppResult<ApiKey>> {
        let name = name.to_string();
        let key_prefix = key_prefix.to_string();
        let key_hash = key_hash.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let key = sqlx::query_as::<_, ApiKey>(
                r#"
                INSERT INTO api_keys (tenant_id, model_id, name, key_prefix, key_hash, rate_limit, expires_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(model_id)
            .bind(&name)
            .bind(&key_prefix)
            .bind(&key_hash)
            .bind(rate_limit)
            .bind(expires_at)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(key)
        })
    }

    /// Auth-by-hash: runs before the tenant is known, so it uses the owner pool.
    /// The hash is a high-entropy secret, so this does not weaken tenant isolation.
    fn get_by_hash(&self, key_hash: &str) -> BoxFuture<'_, AppResult<Option<ApiKey>>> {
        let key_hash = key_hash.to_string();
        Box::pin(async move {
            let key = sqlx::query_as::<_, ApiKey>(
                "SELECT * FROM api_keys WHERE key_hash = $1 AND is_active = TRUE",
            )
            .bind(&key_hash)
            .fetch_optional(&self.db_admin)
            .await?;

            Ok(key)
        })
    }

    fn list_by_model(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Vec<ApiKey>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let keys = sqlx::query_as::<_, ApiKey>(
                r#"
                SELECT * FROM api_keys
                WHERE model_id = $1 AND tenant_id = $2
                ORDER BY created_at DESC
                LIMIT 1000
                "#,
            )
            .bind(model_id)
            .bind(tenant_id)
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(keys)
        })
    }

    fn revoke(&self, tenant_id: Uuid, key_id: Uuid) -> BoxFuture<'_, AppResult<Option<ApiKey>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let key = sqlx::query_as::<_, ApiKey>(
                r#"
                UPDATE api_keys
                SET is_active = FALSE
                WHERE id = $1 AND tenant_id = $2
                RETURNING *
                "#,
            )
            .bind(key_id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(key)
        })
    }

    /// Called on the auth hot path with only the key id (no tenant context yet),
    /// so it uses the owner pool.
    fn update_last_used(&self, key_id: Uuid) -> BoxFuture<'_, AppResult<()>> {
        Box::pin(async move {
            sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
                .bind(key_id)
                .execute(&self.db_admin)
                .await?;

            Ok(())
        })
    }
}
