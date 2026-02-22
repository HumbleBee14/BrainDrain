use chrono::{DateTime, Utc};
use platform_db::models::AuditLog;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;
use uuid::Uuid;

/// Audit log entry returned by API.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct AuditLogResponse {
    pub id: String,
    pub actor_id: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl From<AuditLog> for AuditLogResponse {
    fn from(log: AuditLog) -> Self {
        Self {
            id: log.id.to_string(),
            actor_id: log.actor_id,
            action: log.action,
            resource_type: log.resource_type,
            resource_id: log.resource_id.map(|id| id.to_string()),
            metadata: log.metadata,
            created_at: log.created_at,
        }
    }
}

/// Optional filters for audit log queries.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AuditLogFilterParams {
    #[serde(default)]
    pub offset: i64,
    #[serde(default = "default_limit")]
    pub limit: i64,
    pub resource_type: Option<String>,
    pub resource_id: Option<Uuid>,
}

fn default_limit() -> i64 {
    20
}
