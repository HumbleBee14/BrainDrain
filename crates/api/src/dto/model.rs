use chrono::{DateTime, Utc};
use platform_db::models::Model;
use platform_shared::enums::DeploymentStatus;
use platform_shared::types::EvaluationScores;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Model information returned by API.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ModelResponse {
    pub id: String,
    pub project_id: String,
    pub training_job_id: String,
    pub name: String,
    pub base_model: String,
    pub deployment_status: DeploymentStatus,
    pub eval_scores: Option<EvaluationScores>,
    pub version: i32,
    pub capture_traffic: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request body for model rollback.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct RollbackModelRequest {
    /// The ID of the target version to roll back to (deploy).
    pub target_version_id: String,
}

impl From<Model> for ModelResponse {
    fn from(m: Model) -> Self {
        Self {
            id: m.id.to_string(),
            project_id: m.project_id.to_string(),
            training_job_id: m.training_job_id.to_string(),
            name: m.name,
            base_model: m.base_model,
            deployment_status: m
                .deployment_status
                .parse()
                .unwrap_or(DeploymentStatus::Undeployed),
            eval_scores: serde_json::from_value(m.eval_scores).ok(),
            version: m.version,
            capture_traffic: m.capture_traffic,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}
