use chrono::{DateTime, Utc};
use platform_db::models::Model;
use serde::Serialize;

/// Model information returned by API.
#[derive(Debug, Serialize)]
pub struct ModelResponse {
    pub id: String,
    pub project_id: String,
    pub training_job_id: String,
    pub name: String,
    pub base_model: String,
    pub deployment_status: String,
    pub eval_scores: serde_json::Value,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Model> for ModelResponse {
    fn from(m: Model) -> Self {
        Self {
            id: m.id.to_string(),
            project_id: m.project_id.to_string(),
            training_job_id: m.training_job_id.to_string(),
            name: m.name,
            base_model: m.base_model,
            deployment_status: m.deployment_status,
            eval_scores: m.eval_scores,
            version: m.version,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}
