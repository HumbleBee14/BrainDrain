use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Tenant {
    pub id: Uuid,
    pub clerk_org_id: String,
    pub name: String,
    pub plan: String,
    pub settings: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub task_type: Option<String>,
    pub config: serde_json::Value,
    pub status: String,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub filename: String,
    pub file_size: i64,
    pub mime_type: String,
    pub storage_path: String,
    pub status: String,
    pub parse_quality: Option<f64>,
    pub page_count: Option<i32>,
    pub language: Option<String>,
    pub domain: Option<String>,
    pub metadata: serde_json::Value,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Dataset {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub storage_path: Option<String>,
    pub format: String,
    pub status: String,
    pub pair_count: Option<i32>,
    pub stats: serde_json::Value,
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TrainingJob {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub dataset_id: Uuid,
    pub base_model: String,
    pub method: String,
    pub mode: String,
    pub hyperparams: serde_json::Value,
    pub gpu_class: Option<String>,
    pub status: String,
    pub cost_estimate: Option<f64>,
    pub actual_cost: Option<f64>,
    pub metrics: serde_json::Value,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub temporal_workflow_id: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Model {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub training_job_id: Uuid,
    pub name: String,
    pub base_model: String,
    pub adapter_path: Option<String>,
    pub adapter_size_bytes: Option<i64>,
    pub deployment_status: String,
    pub deployment_config: serde_json::Value,
    pub eval_scores: serde_json::Value,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Evaluation {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub model_id: Uuid,
    pub status: String,
    pub scores: serde_json::Value,
    pub report: serde_json::Value,
    pub temporal_workflow_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub model_id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub key_hash: String,
    pub rate_limit: i32,
    pub is_active: bool,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BillingEvent {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub operation: String,
    pub resource_id: Option<Uuid>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub gpu_seconds: Option<i32>,
    pub cost_usd: f64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}
