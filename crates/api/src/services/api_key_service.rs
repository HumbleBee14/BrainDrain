use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::api_key::{ApiKeyResponse, CreateApiKeyRequest, CreateApiKeyResponse};
use crate::error::{AppError, AppResult};
use crate::repositories::api_key_repo::ApiKeyRepo;
use crate::repositories::model_repo::ModelRepo;

/// Business logic for API key operations.
pub struct ApiKeyService;

/// Result of authenticating an API key.
pub struct AuthenticatedApiKey {
    pub key_id: Uuid,
    pub tenant_id: Uuid,
    pub model_id: Uuid,
}

impl ApiKeyService {
    /// Create a new API key for a model.
    pub async fn create(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
        req: CreateApiKeyRequest,
    ) -> AppResult<CreateApiKeyResponse> {
        // Verify model exists and belongs to tenant
        ModelRepo::get_by_id(db, tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        if req.name.trim().is_empty() {
            return Err(AppError::BadRequest {
                message: "API key name is required".to_string(),
            });
        }

        // Generate key: pl_sk_ + 32 random bytes base64url
        let raw_key = generate_key();
        let key_hash = hash_key(&raw_key);
        let key_prefix = raw_key[..14].to_string(); // "pl_sk_" + 8 chars

        let rate_limit = req.rate_limit.unwrap_or(60);
        let expires_at = req
            .expires_in_days
            .map(|days| Utc::now() + Duration::days(days));

        let key_record = ApiKeyRepo::create(
            db,
            tenant_id,
            model_id,
            &req.name,
            &key_prefix,
            &key_hash,
            rate_limit,
            expires_at,
        )
        .await?;

        tracing::info!(
            model_id = %model_id,
            key_prefix = %key_prefix,
            "API key created"
        );

        Ok(CreateApiKeyResponse {
            id: key_record.id.to_string(),
            name: key_record.name,
            key: raw_key,
            key_prefix: key_record.key_prefix,
            rate_limit: key_record.rate_limit,
            expires_at: key_record.expires_at,
            created_at: key_record.created_at,
        })
    }

    /// List API keys for a model (without full key — only prefix).
    pub async fn list(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<Vec<ApiKeyResponse>> {
        let keys = ApiKeyRepo::list_by_model(db, tenant_id, model_id).await?;
        Ok(keys.into_iter().map(Into::into).collect())
    }

    /// Revoke an API key.
    pub async fn revoke(db: &PgPool, tenant_id: Uuid, key_id: Uuid) -> AppResult<ApiKeyResponse> {
        let key = ApiKeyRepo::revoke(db, tenant_id, key_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "API key not found".to_string(),
            })?;

        tracing::info!(key_id = %key_id, "API key revoked");
        Ok(key.into())
    }

    /// Authenticate a raw API key. Returns the key's identity on success.
    pub async fn authenticate(
        db: &PgPool,
        redis: &mut redis::aio::ConnectionManager,
        raw_key: &str,
    ) -> AppResult<AuthenticatedApiKey> {
        let key_hash = hash_key(raw_key);

        let key = ApiKeyRepo::get_by_hash(db, &key_hash)
            .await?
            .ok_or(AppError::Unauthorized)?;

        // Check expiry
        if let Some(expires_at) = key.expires_at
            && Utc::now() > expires_at
        {
            return Err(AppError::Unauthorized);
        }

        // Rate limiting: per-minute sliding window
        let minute = Utc::now().format("%Y%m%d%H%M").to_string();
        let rl_key = format!(
            "{}{}:{}",
            platform_shared::constants::REDIS_RATE_LIMIT_PREFIX,
            key.id,
            minute
        );

        let count: i64 = redis::cmd("INCR")
            .arg(&rl_key)
            .query_async(redis)
            .await
            .unwrap_or(1);

        if count == 1 {
            // Set TTL on first request in this window
            let _: Result<(), redis::RedisError> = redis::cmd("EXPIRE")
                .arg(&rl_key)
                .arg(60)
                .query_async(redis)
                .await;
        }

        if count > key.rate_limit as i64 {
            return Err(AppError::RateLimited);
        }

        // Update last_used_at (fire-and-forget)
        let db_clone = db.clone();
        let key_id = key.id;
        tokio::spawn(async move {
            let _ = ApiKeyRepo::update_last_used(&db_clone, key_id).await;
        });

        Ok(AuthenticatedApiKey {
            key_id: key.id,
            tenant_id: key.tenant_id,
            model_id: key.model_id,
        })
    }
}

/// Generate a new API key: `pl_sk_` + 32 random bytes as base64url.
fn generate_key() -> String {
    use rand::RngCore;

    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes);
    format!("pl_sk_{encoded}")
}

/// Hash an API key using SHA-256.
fn hash_key(raw_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_format_is_correct() {
        let key = generate_key();
        assert!(key.starts_with("pl_sk_"));
        assert!(key.len() > 14); // prefix + base64
    }

    #[test]
    fn hash_is_consistent() {
        let key = "pl_sk_test_key_1234567890";
        let hash1 = hash_key(key);
        let hash2 = hash_key(key);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn different_keys_different_hashes() {
        let hash1 = hash_key("pl_sk_key_a");
        let hash2 = hash_key("pl_sk_key_b");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn hash_is_hex_string() {
        let hash = hash_key("pl_sk_test");
        assert_eq!(hash.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
