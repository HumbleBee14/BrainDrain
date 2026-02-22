use chrono::{DateTime, Utc};
use platform_db::models::Evaluation;
use platform_shared::enums::EvaluationStatus;
use platform_shared::types::EvaluationScores;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Request to create a new evaluation.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CreateEvaluationRequest {
    #[ts(optional)]
    pub judge_model: Option<String>,
    #[ts(optional)]
    pub judge_api_base: Option<String>,
}

/// Evaluation information returned by API.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct EvaluationResponse {
    pub id: String,
    pub model_id: String,
    pub status: EvaluationStatus,
    pub scores: Option<EvaluationScores>,
    #[schema(value_type = Object)]
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
            status: e.status.parse().unwrap_or(EvaluationStatus::Running),
            scores: serde_json::from_value(e.scores).ok(),
            report: e.report,
            started_at: e.started_at,
            completed_at: e.completed_at,
            created_at: e.created_at,
            updated_at: e.updated_at,
        }
    }
}
