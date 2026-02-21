use chrono::{DateTime, Utc};
use platform_db::models::Evaluation;
use serde::{Deserialize, Serialize};

/// Request to create a new evaluation.
#[derive(Debug, Deserialize)]
pub struct CreateEvaluationRequest {
    pub judge_model: Option<String>,
    pub judge_api_base: Option<String>,
}

/// Evaluation information returned by API.
#[derive(Debug, Serialize)]
pub struct EvaluationResponse {
    pub id: String,
    pub model_id: String,
    pub status: String,
    pub scores: serde_json::Value,
    pub report: serde_json::Value,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Evaluation> for EvaluationResponse {
    fn from(e: Evaluation) -> Self {
        Self {
            id: e.id.to_string(),
            model_id: e.model_id.to_string(),
            status: e.status,
            scores: e.scores,
            report: e.report,
            started_at: e.started_at,
            completed_at: e.completed_at,
            created_at: e.created_at,
            updated_at: e.updated_at,
        }
    }
}
