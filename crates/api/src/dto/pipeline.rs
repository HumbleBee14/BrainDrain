use serde::{Deserialize, Serialize};

/// Response from triggering document parsing.
#[derive(Debug, Serialize)]
pub struct TriggerParseResponse {
    pub workflow_id: String,
    pub document_count: usize,
}

/// Request body for triggering data refinement.
#[derive(Debug, Deserialize)]
pub struct TriggerRefineRequest {
    pub task_type: Option<String>,
    #[serde(default)]
    pub config: serde_json::Value,
}

/// Response from triggering data refinement.
#[derive(Debug, Serialize)]
pub struct TriggerRefineResponse {
    pub workflow_id: String,
    pub document_count: usize,
}

/// Aggregate pipeline status for a project.
#[derive(Debug, Serialize)]
pub struct ProjectPipelineStatus {
    pub project_id: String,
    pub documents: DocumentStatusCounts,
    pub datasets: DatasetStatusCounts,
}

#[derive(Debug, Serialize)]
pub struct DocumentStatusCounts {
    pub total: i64,
    pub uploaded: i64,
    pub parsing: i64,
    pub parsed: i64,
    pub failed: i64,
}

#[derive(Debug, Serialize)]
pub struct DatasetStatusCounts {
    pub total: i64,
    pub generating: i64,
    pub review_pending: i64,
    pub approved: i64,
}
