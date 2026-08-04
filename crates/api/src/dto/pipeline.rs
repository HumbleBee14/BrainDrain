use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::services::teacher::config::TeacherConfigDto;
use crate::services::teacher::policy::ProviderPolicy;

/// Response from triggering document parsing.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct TriggerParseResponse {
    pub workflow_id: String,
    pub document_count: usize,
}

/// Request body for triggering data refinement.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct TriggerRefineRequest {
    #[ts(optional)]
    pub task_type: Option<String>,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub config: serde_json::Value,
    /// Distillation: the external model that writes the training examples.
    #[ts(optional)]
    pub teacher: Option<TeacherConfigDto>,
}

/// Response from triggering data refinement.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct TriggerRefineResponse {
    pub workflow_id: String,
    pub document_count: usize,
    /// Provider policy of the teacher this run uses (badge state for the UI).
    pub teacher_policy: Option<ProviderPolicy>,
}

/// Response from triggering training.
#[allow(dead_code)]
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct TriggerTrainResponse {
    pub workflow_id: String,
    pub training_job_id: String,
}

/// Request body for triggering the full pipeline (one-click fine-tune).
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct TriggerFullPipelineRequest {
    /// Task type for refinement (defaults to project's task_type or "question_answering")
    #[ts(optional)]
    pub task_type: Option<String>,
    /// Base model to fine-tune (e.g. "unsloth/Llama-3.2-1B-Instruct")
    pub base_model: String,
    /// Training configuration: method, mode, hyperparams, gpu_class, auto_deploy
    #[serde(default)]
    #[schema(value_type = Object)]
    pub training_config: serde_json::Value,
    /// Distillation: the external model that writes the training examples.
    /// Required when `training_config.mode` is `"distill"`.
    #[ts(optional)]
    pub teacher: Option<TeacherConfigDto>,
}

/// Response from triggering the full pipeline.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct TriggerFullPipelineResponse {
    pub workflow_id: String,
    pub document_count: usize,
    /// Provider policy of the teacher this run uses (badge state for the UI).
    pub teacher_policy: Option<ProviderPolicy>,
}

/// Aggregate pipeline status for a project.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ProjectPipelineStatus {
    pub project_id: String,
    pub documents: DocumentStatusCounts,
    pub datasets: DatasetStatusCounts,
    pub training_jobs: TrainingJobStatusCounts,
    pub models: ModelStatusCounts,
    pub evaluations: EvaluationStatusCounts,
}

#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DocumentStatusCounts {
    pub total: i64,
    pub uploaded: i64,
    pub parsing: i64,
    pub parsed: i64,
    pub failed: i64,
}

#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DatasetStatusCounts {
    pub total: i64,
    pub generating: i64,
    pub review_pending: i64,
    pub approved: i64,
    pub failed: i64,
}

#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct TrainingJobStatusCounts {
    pub total: i64,
    pub pending: i64,
    pub training: i64,
    pub completed: i64,
    pub failed: i64,
}

#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ModelStatusCounts {
    pub total: i64,
    pub undeployed: i64,
    pub active: i64,
}

#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct EvaluationStatusCounts {
    pub total: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
}
