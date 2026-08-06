use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// High-level platform statistics across all projects for a tenant.
#[derive(Debug, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DashboardStats {
    pub total_projects: i64,
    pub total_documents: i64,
    pub total_training_jobs: i64,
    pub active_training_jobs: i64,
    pub total_models: i64,
    pub deployed_models: i64,
    pub total_evaluations: i64,
}

/// Billing usage summary with daily cost breakdown.
#[derive(Debug, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UsageSummary {
    pub total_cost_usd: f64,
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
    pub total_events: i64,
    pub cost_by_day: Vec<DailyCost>,
    pub cost_by_operation: Vec<OperationCost>,
}

/// Cost for a single day.
#[derive(Debug, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DailyCost {
    pub date: String,
    pub cost_usd: f64,
}

/// Lifetime cost attributed to one billed operation type.
#[derive(Debug, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct OperationCost {
    pub operation: String,
    pub cost_usd: f64,
    pub events: i64,
}

/// Recent activity entry from the audit log.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ActivityEntry {
    pub id: String,
    pub actor_id: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub created_at: String,
}
