use chrono::{DateTime, Utc};
use platform_db::models::ApiKey;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Request to create a new API key.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[ts(optional)]
    pub rate_limit: Option<i32>,
    #[ts(optional)]
    pub expires_in_days: Option<i64>,
}

/// Response returned when an API key is created.
/// Includes the full key — only shown once.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct CreateApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key: String,
    pub key_prefix: String,
    pub rate_limit: i32,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// API key information returned by list/get endpoints.
/// Does NOT include the full key (it's hashed in DB).
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ApiKeyResponse {
    pub id: String,
    pub model_id: String,
    pub name: String,
    pub key_prefix: String,
    pub rate_limit: i32,
    pub is_active: bool,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl From<ApiKey> for ApiKeyResponse {
    fn from(k: ApiKey) -> Self {
        Self {
            id: k.id.to_string(),
            model_id: k.model_id.to_string(),
            name: k.name,
            key_prefix: k.key_prefix,
            rate_limit: k.rate_limit,
            is_active: k.is_active,
            last_used_at: k.last_used_at,
            expires_at: k.expires_at,
            created_at: k.created_at,
        }
    }
}
