use chrono::{DateTime, Utc};
use platform_db::models::BillingEvent;
use serde::Serialize;

/// Billing event returned by API.
#[derive(Debug, Serialize)]
pub struct BillingEventResponse {
    pub id: String,
    pub operation: String,
    pub resource_id: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub gpu_seconds: Option<i32>,
    pub cost_usd: f64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl From<BillingEvent> for BillingEventResponse {
    fn from(e: BillingEvent) -> Self {
        Self {
            id: e.id.to_string(),
            operation: e.operation,
            resource_id: e.resource_id.map(|id| id.to_string()),
            tokens_in: e.tokens_in,
            tokens_out: e.tokens_out,
            gpu_seconds: e.gpu_seconds,
            cost_usd: e.cost_usd,
            metadata: e.metadata,
            created_at: e.created_at,
        }
    }
}
