use chrono::{DateTime, Utc};
use platform_db::models::TrainingJob;
use platform_shared::enums::{TrainingJobStatus, TrainingMethod, TrainingMode};
use platform_shared::types::{Hyperparams, TrainingMetrics};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::services::teacher::config::{
    TeacherConfigDto, TeacherProvenance, provenance_from_config,
};
use crate::services::teacher::extraction::DistillOptionsDto;

/// Request to create a new training job.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CreateTrainingJobRequest {
    pub dataset_id: String,
    pub base_model: String,
    #[ts(optional)]
    pub method: Option<TrainingMethod>,
    #[ts(optional)]
    pub mode: Option<TrainingMode>,
    #[ts(optional)]
    pub hyperparams: Option<Hyperparams>,
    #[ts(optional)]
    pub gpu_class: Option<String>,
    /// Distill mode: the teacher recorded on the job. Optional — when absent
    /// the teacher is taken from the dataset's provenance.
    #[ts(optional)]
    pub teacher: Option<TeacherConfigDto>,
    /// Distill mode: fidelity options. Absent means the text path.
    #[ts(optional)]
    pub distill: Option<DistillOptionsDto>,
}

/// Training job information returned by API.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct TrainingJobResponse {
    pub id: String,
    pub project_id: String,
    pub dataset_id: String,
    pub base_model: String,
    pub method: TrainingMethod,
    pub mode: TrainingMode,
    pub hyperparams: Hyperparams,
    pub gpu_class: Option<String>,
    pub status: TrainingJobStatus,
    pub cost_estimate: Option<f64>,
    pub actual_cost: Option<f64>,
    pub metrics: TrainingMetrics,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    /// Distill mode: teacher provenance (host + model only — never the key).
    pub teacher: Option<TeacherProvenance>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Cost estimate breakdown returned by the estimate endpoint.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct CostEstimateResponse {
    pub cost_estimate: f64,
    pub estimated_hours: f64,
    pub gpu_class: String,
    pub gpu_rate_per_hour: f64,
}

impl From<TrainingJob> for TrainingJobResponse {
    fn from(j: TrainingJob) -> Self {
        Self {
            id: j.id.to_string(),
            project_id: j.project_id.to_string(),
            dataset_id: j.dataset_id.to_string(),
            base_model: j.base_model,
            method: j.method.parse().unwrap_or(TrainingMethod::Qlora),
            mode: j.mode.parse().unwrap_or(TrainingMode::Quick),
            hyperparams: serde_json::from_value(j.hyperparams).unwrap_or_default(),
            gpu_class: j.gpu_class,
            status: j.status.parse().unwrap_or(TrainingJobStatus::Pending),
            cost_estimate: j.cost_estimate,
            actual_cost: j.actual_cost,
            metrics: serde_json::from_value(j.metrics).unwrap_or_default(),
            started_at: j.started_at,
            completed_at: j.completed_at,
            error_message: j.error_message,
            teacher: j.teacher_config.as_ref().and_then(provenance_from_config),
            created_at: j.created_at,
            updated_at: j.updated_at,
        }
    }
}
