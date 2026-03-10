use chrono::{DateTime, Utc};
use platform_shared::enums::ProjectStatus;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;
use uuid::Uuid;

/// Request body for creating a new project.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CreateProjectRequest {
    pub name: String,
    #[ts(optional)]
    pub description: Option<String>,
    #[ts(optional)]
    pub task_type: Option<String>,
}

/// Request body for updating an existing project.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateProjectRequest {
    #[ts(optional)]
    pub name: Option<String>,
    #[ts(optional)]
    pub description: Option<String>,
    #[ts(optional)]
    pub task_type: Option<String>,
}

/// Request body for updating a project's status.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateProjectStatusRequest {
    pub status: ProjectStatus,
}

/// API response for a project.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ProjectResponse {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub task_type: Option<String>,
    pub status: ProjectStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<platform_db::models::Project> for ProjectResponse {
    fn from(p: platform_db::models::Project) -> Self {
        Self {
            id: p.id,
            name: p.name,
            description: p.description,
            task_type: p.task_type,
            status: p.status.parse().unwrap_or(ProjectStatus::Created),
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}
