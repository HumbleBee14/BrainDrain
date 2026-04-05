use chrono::{DateTime, Utc};
use platform_db::models::InferenceInstance;
use platform_shared::enums::{InferenceInstanceHealthStatus, InferenceInstanceLifecycleState};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CreateInferenceInstanceRequest {
    pub name: String,
    pub base_url: String,
    pub backend_type: String,
    #[serde(default)]
    pub gpu_class: Option<String>,
    pub base_model: String,
    #[serde(default = "default_max_adapters")]
    pub max_adapters: i32,
    #[serde(default = "default_metadata")]
    pub metadata: serde_json::Value,
}

fn default_max_adapters() -> i32 {
    4
}

fn default_metadata() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateInferenceInstanceLifecycleRequest {
    pub lifecycle_state: InferenceInstanceLifecycleState,
}

#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct InferenceInstanceResponse {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub backend_type: String,
    pub gpu_class: Option<String>,
    pub base_model: String,
    pub max_adapters: i32,
    pub active_adapter_count: i32,
    pub health_status: InferenceInstanceHealthStatus,
    pub lifecycle_state: InferenceInstanceLifecycleState,
    pub last_health_check_at: Option<DateTime<Utc>>,
    pub last_healthy_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<InferenceInstance> for InferenceInstanceResponse {
    fn from(instance: InferenceInstance) -> Self {
        Self {
            id: instance.id.to_string(),
            name: instance.name,
            base_url: instance.base_url,
            backend_type: instance.backend_type,
            gpu_class: instance.gpu_class,
            base_model: instance.base_model,
            max_adapters: instance.max_adapters,
            active_adapter_count: instance.active_adapter_count,
            health_status: instance
                .health_status
                .parse()
                .unwrap_or(InferenceInstanceHealthStatus::Unknown),
            lifecycle_state: instance
                .lifecycle_state
                .parse()
                .unwrap_or(InferenceInstanceLifecycleState::Ready),
            last_health_check_at: instance.last_health_check_at,
            last_healthy_at: instance.last_healthy_at,
            metadata: instance.metadata,
            created_at: instance.created_at,
            updated_at: instance.updated_at,
        }
    }
}
