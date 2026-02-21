use platform_db::models::ApiKey;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Repository for API key database operations.
///
/// API keys are looked up by hash (no tenant_id needed for auth by hash).
/// All management queries require `tenant_id`.
pub struct ApiKeyRepo;

impl ApiKeyRepo {
    /// Create a new API key record.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
        name: &str,
        key_prefix: &str,
        key_hash: &str,
        rate_limit: i32,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<ApiKey, AppError> {
        let key = sqlx::query_as::<_, ApiKey>(
            r#"
            INSERT INTO api_keys (tenant_id, model_id, name, key_prefix, key_hash, rate_limit, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(model_id)
        .bind(name)
        .bind(key_prefix)
        .bind(key_hash)
        .bind(rate_limit)
        .bind(expires_at)
        .fetch_one(db)
        .await?;

        Ok(key)
    }

    /// Look up an active API key by its hash. No tenant_id — auth by hash.
    pub async fn get_by_hash(db: &PgPool, key_hash: &str) -> Result<Option<ApiKey>, AppError> {
        let key = sqlx::query_as::<_, ApiKey>(
            "SELECT * FROM api_keys WHERE key_hash = $1 AND is_active = TRUE",
        )
        .bind(key_hash)
        .fetch_optional(db)
        .await?;

        Ok(key)
    }

    /// List API keys for a model within a tenant.
    pub async fn list_by_model(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> Result<Vec<ApiKey>, AppError> {
        let keys = sqlx::query_as::<_, ApiKey>(
            r#"
            SELECT * FROM api_keys
            WHERE model_id = $1 AND tenant_id = $2
            ORDER BY created_at DESC
            "#,
        )
        .bind(model_id)
        .bind(tenant_id)
        .fetch_all(db)
        .await?;

        Ok(keys)
    }

    /// Revoke an API key (soft delete by setting is_active = false).
    pub async fn revoke(
        db: &PgPool,
        tenant_id: Uuid,
        key_id: Uuid,
    ) -> Result<Option<ApiKey>, AppError> {
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
        .fetch_optional(db)
        .await?;

        Ok(key)
    }

    /// Update last_used_at timestamp.
    pub async fn update_last_used(db: &PgPool, key_id: Uuid) -> Result<(), AppError> {
        sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
            .bind(key_id)
            .execute(db)
            .await?;

        Ok(())
    }
}
