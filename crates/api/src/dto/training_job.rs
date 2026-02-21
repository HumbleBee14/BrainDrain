use chrono::{DateTime, Utc};
use platform_db::models::TrainingJob;
use serde::{Deserialize, Serialize};

/// Request to create a new training job.
#[derive(Debug, Deserialize)]
pub struct CreateTrainingJobRequest {
    pub dataset_id: String,
    pub base_model: String,
    pub method: Option<String>,
    pub mode: Option<String>,
    pub hyperparams: Option<serde_json::Value>,
    pub gpu_class: Option<String>,
}

/// Training job information returned by API.
#[derive(Debug, Serialize)]
pub struct TrainingJobResponse {
    pub id: String,
    pub project_id: String,
    pub dataset_id: String,
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
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<TrainingJob> for TrainingJobResponse {
    fn from(j: TrainingJob) -> Self {
        Self {
            id: j.id.to_string(),
            project_id: j.project_id.to_string(),
            dataset_id: j.dataset_id.to_string(),
            base_model: j.base_model,
            method: j.method,
            mode: j.mode,
            hyperparams: j.hyperparams,
            gpu_class: j.gpu_class,
            status: j.status,
            cost_estimate: j.cost_estimate,
            actual_cost: j.actual_cost,
            metrics: j.metrics,
            started_at: j.started_at,
            completed_at: j.completed_at,
            error_message: j.error_message,
            created_at: j.created_at,
            updated_at: j.updated_at,
        }
    }
}
