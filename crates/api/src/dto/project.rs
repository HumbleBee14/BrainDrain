use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request body for creating a new project.
#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    pub description: Option<String>,
    pub task_type: Option<String>,
}

/// Request body for updating an existing project.
#[derive(Debug, Deserialize)]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub task_type: Option<String>,
}

/// API response for a project.
#[derive(Debug, Serialize)]
pub struct ProjectResponse {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub task_type: Option<String>,
    pub status: String,
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
            status: p.status,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}
