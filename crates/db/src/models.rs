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
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub plan_limits: serde_json::Value,
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
pub struct DataGuide {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub task_type: String,
    pub status: String,
    pub guidance: String,
    pub system_prompt: String,
    pub facets: serde_json::Value,
    pub preview_samples: serde_json::Value,
    pub refinement_history: serde_json::Value,
    pub config: serde_json::Value,
    pub dataset_id: Option<Uuid>,
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
    pub inference_instance_id: Option<Uuid>,
    pub deployment_config: serde_json::Value,
    pub eval_scores: serde_json::Value,
    pub version: i32,
    pub capture_traffic: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct InferenceSample {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub model_id: Uuid,
    pub api_key_id: Option<Uuid>,
    pub messages: serde_json::Value,
    pub response: String,
    pub rating: Option<String>,
    pub rating_comment: Option<String>,
    pub promoted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct InferenceInstance {
    pub id: Uuid,
    pub name: String,
    pub base_url: String,
    pub backend_type: String,
    pub gpu_class: Option<String>,
    pub base_model: String,
    pub max_adapters: i32,
    pub active_adapter_count: i32,
    pub health_status: String,
    pub lifecycle_state: String,
    pub last_health_check_at: Option<DateTime<Utc>>,
    pub last_healthy_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
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

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub actor_id: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<Uuid>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TeamMember {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: String,
    pub email: String,
    pub role: String,
    pub invited_by: Option<String>,
    pub joined_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Invitation {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub email: String,
    pub role: String,
    pub token: String,
    pub invited_by: String,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct NotificationPreference {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub channel: String,
    pub event_type: String,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct NotificationDelivery {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub preference_id: Uuid,
    pub event_type: String,
    pub channel: String,
    pub payload: serde_json::Value,
    pub status: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
    pub read_at: Option<DateTime<Utc>>,
    pub next_retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ModelExport {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub model_id: Uuid,
    pub format: String,
    pub quant_type: String,
    pub status: String,
    pub storage_path: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}
