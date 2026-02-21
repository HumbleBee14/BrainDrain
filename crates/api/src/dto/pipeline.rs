use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Response from triggering document parsing.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct TriggerParseResponse {
    pub workflow_id: String,
    pub document_count: usize,
}

/// Request body for triggering data refinement.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct TriggerRefineRequest {
    #[ts(optional)]
    pub task_type: Option<String>,
    #[serde(default)]
    pub config: serde_json::Value,
}

/// Response from triggering data refinement.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct TriggerRefineResponse {
    pub workflow_id: String,
    pub document_count: usize,
}

/// Response from triggering training.
#[allow(dead_code)]
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct TriggerTrainResponse {
    pub workflow_id: String,
    pub training_job_id: String,
}

/// Aggregate pipeline status for a project.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ProjectPipelineStatus {
    pub project_id: String,
    pub documents: DocumentStatusCounts,
    pub datasets: DatasetStatusCounts,
    pub training_jobs: TrainingJobStatusCounts,
    pub models: ModelStatusCounts,
    pub evaluations: EvaluationStatusCounts,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct DocumentStatusCounts {
    pub total: i64,
    pub uploaded: i64,
    pub parsing: i64,
    pub parsed: i64,
    pub failed: i64,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct DatasetStatusCounts {
    pub total: i64,
    pub generating: i64,
    pub review_pending: i64,
    pub approved: i64,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct TrainingJobStatusCounts {
    pub total: i64,
    pub pending: i64,
    pub training: i64,
    pub completed: i64,
    pub failed: i64,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ModelStatusCounts {
    pub total: i64,
    pub undeployed: i64,
    pub active: i64,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct EvaluationStatusCounts {
    pub total: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
}
