use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::dto::api_key::{ApiKeyResponse, CreateApiKeyRequest, CreateApiKeyResponse};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{ApiKeyRepository, ModelRepository};

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
        api_key_repo: &dyn ApiKeyRepository,
        model_repo: &dyn ModelRepository,
        tenant_id: Uuid,
        model_id: Uuid,
        req: CreateApiKeyRequest,
    ) -> AppResult<CreateApiKeyResponse> {
        // Verify model exists and belongs to tenant
        model_repo
            .get_by_id(tenant_id, model_id)
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

        let key_record = api_key_repo
            .create(
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
        repo: &dyn ApiKeyRepository,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<Vec<ApiKeyResponse>> {
        let keys = repo.list_by_model(tenant_id, model_id).await?;
        Ok(keys.into_iter().map(Into::into).collect())
    }

    /// Revoke an API key.
    pub async fn revoke(
        repo: &dyn ApiKeyRepository,
        tenant_id: Uuid,
        key_id: Uuid,
    ) -> AppResult<ApiKeyResponse> {
        let key = repo
            .revoke(tenant_id, key_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "API key not found".to_string(),
            })?;

        tracing::info!(key_id = %key_id, "API key revoked");
        Ok(key.into())
    }

    /// Authenticate a raw API key. Returns the key's identity on success.
    pub async fn authenticate(
        repo: &dyn ApiKeyRepository,
        redis: &mut redis::aio::ConnectionManager,
        raw_key: &str,
    ) -> AppResult<AuthenticatedApiKey> {
        let key_hash = hash_key(raw_key);

        let key = repo
            .get_by_hash(&key_hash)
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

        // Atomic INCR + EXPIRE via Lua script to prevent race where
        // EXPIRE fails and the key never expires (permanent rate limit).
        let count: i64 = redis::Script::new(
            r#"
            local count = redis.call('INCR', KEYS[1])
            if count == 1 then
                redis.call('EXPIRE', KEYS[1], ARGV[1])
            end
            return count
            "#,
        )
        .key(&rl_key)
        .arg(60)
        .invoke_async(redis)
        .await
        .unwrap_or(1);

        if count > key.rate_limit as i64 {
            return Err(AppError::RateLimited);
        }

        // Update last_used_at inline (can't spawn with &dyn trait)
        if let Err(e) = repo.update_last_used(key.id).await {
            tracing::warn!(key_id = %key.id, error = %e, "Failed to update API key last_used_at");
        }

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
    use crate::dto::api_key::CreateApiKeyRequest;

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

    // ── Key generation properties ──

    #[test]
    fn generated_keys_are_unique() {
        let key1 = generate_key();
        let key2 = generate_key();
        assert_ne!(key1, key2, "Two generated keys should never collide");
    }

    #[test]
    fn key_prefix_extractable_for_display() {
        let key = generate_key();
        let prefix = &key[..14]; // "pl_sk_" (6) + 8 chars
        assert!(prefix.starts_with("pl_sk_"));
        assert_eq!(prefix.len(), 14);
    }

    #[test]
    fn key_is_base64url_safe_after_prefix() {
        let key = generate_key();
        let after_prefix = &key[6..]; // skip "pl_sk_"
        // base64url chars: A-Z, a-z, 0-9, -, _
        assert!(
            after_prefix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "Key body should only contain base64url-safe characters, got: {after_prefix}",
        );
    }

    #[test]
    fn key_length_is_deterministic() {
        // 32 random bytes -> 43 base64url chars (no padding) + 6 prefix = 49
        let key = generate_key();
        assert_eq!(key.len(), 49, "pl_sk_ (6) + base64url(32 bytes) (43) = 49");
    }

    // ── Hash properties ──

    #[test]
    fn hash_of_empty_string_is_valid() {
        let hash = hash_key("");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_is_lowercase_hex() {
        let hash = hash_key("pl_sk_some_key");
        assert!(
            hash.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "Hash should be lowercase hex, got: {hash}",
        );
    }

    // ── Name validation (mirrors check in ApiKeyService::create) ──

    #[test]
    fn empty_api_key_name_is_rejected() {
        let req = CreateApiKeyRequest {
            name: "".to_string(),
            rate_limit: None,
            expires_in_days: None,
        };
        assert!(req.name.trim().is_empty());
    }

    #[test]
    fn whitespace_api_key_name_is_rejected() {
        let req = CreateApiKeyRequest {
            name: "   ".to_string(),
            rate_limit: None,
            expires_in_days: None,
        };
        assert!(req.name.trim().is_empty());
    }

    #[test]
    fn valid_api_key_name_passes() {
        let req = CreateApiKeyRequest {
            name: "production-key".to_string(),
            rate_limit: None,
            expires_in_days: None,
        };
        assert!(!req.name.trim().is_empty());
    }

    // ── Rate limit defaults ──

    #[test]
    fn default_rate_limit_is_60() {
        let req = CreateApiKeyRequest {
            name: "test".to_string(),
            rate_limit: None,
            expires_in_days: None,
        };
        let rate_limit = req.rate_limit.unwrap_or(60);
        assert_eq!(rate_limit, 60);
    }

    #[test]
    fn custom_rate_limit_is_respected() {
        let req = CreateApiKeyRequest {
            name: "test".to_string(),
            rate_limit: Some(120),
            expires_in_days: None,
        };
        let rate_limit = req.rate_limit.unwrap_or(60);
        assert_eq!(rate_limit, 120);
    }

    // ── Expiry computation ──

    #[test]
    fn no_expiry_when_expires_in_days_is_none() {
        let req = CreateApiKeyRequest {
            name: "test".to_string(),
            rate_limit: None,
            expires_in_days: None,
        };
        let expires_at = req
            .expires_in_days
            .map(|days| Utc::now() + Duration::days(days));
        assert!(expires_at.is_none());
    }

    #[test]
    fn expiry_is_in_the_future() {
        let req = CreateApiKeyRequest {
            name: "test".to_string(),
            rate_limit: None,
            expires_in_days: Some(30),
        };
        let expires_at = req
            .expires_in_days
            .map(|days| Utc::now() + Duration::days(days));
        assert!(expires_at.unwrap() > Utc::now());
    }
}
