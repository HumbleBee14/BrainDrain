use platform_db::models::ApiKey;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{ApiKeyRepository, BoxFuture};

/// PostgreSQL implementation of the API key repository.
///
/// API keys are looked up by hash (no tenant_id needed for auth by hash).
/// All management queries require `tenant_id`.
pub struct PgApiKeyRepo {
    db: PgPool,
}

impl PgApiKeyRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
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
            .fetch_one(&self.db)
            .await?;

            Ok(key)
        })
    }

    fn get_by_hash(&self, key_hash: &str) -> BoxFuture<'_, AppResult<Option<ApiKey>>> {
        let key_hash = key_hash.to_string();
        Box::pin(async move {
            let key = sqlx::query_as::<_, ApiKey>(
                "SELECT * FROM api_keys WHERE key_hash = $1 AND is_active = TRUE",
            )
            .bind(&key_hash)
            .fetch_optional(&self.db)
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
            let keys = sqlx::query_as::<_, ApiKey>(
                r#"
                SELECT * FROM api_keys
                WHERE model_id = $1 AND tenant_id = $2
                ORDER BY created_at DESC
                "#,
            )
            .bind(model_id)
            .bind(tenant_id)
            .fetch_all(&self.db)
            .await?;

            Ok(keys)
        })
    }

    fn revoke(&self, tenant_id: Uuid, key_id: Uuid) -> BoxFuture<'_, AppResult<Option<ApiKey>>> {
        Box::pin(async move {
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
            .fetch_optional(&self.db)
            .await?;

            Ok(key)
        })
    }

    fn update_last_used(&self, key_id: Uuid) -> BoxFuture<'_, AppResult<()>> {
        Box::pin(async move {
            sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
                .bind(key_id)
                .execute(&self.db)
                .await?;

            Ok(())
        })
    }
}
