use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::common::PaginatedResponse;
use crate::dto::project::{CreateProjectRequest, ProjectResponse, UpdateProjectRequest};
use crate::error::{AppError, AppResult};
use crate::repositories::project_repo::ProjectRepo;

/// Business logic for project operations.
///
/// Thin layer over the repository that adds validation and event publishing.
/// As the platform grows, cross-cutting concerns (billing, notifications) live here.
pub struct ProjectService;

impl ProjectService {
    pub async fn create(
        db: &PgPool,
        tenant_id: Uuid,
        req: CreateProjectRequest,
    ) -> AppResult<ProjectResponse> {
        if req.name.trim().is_empty() {
            return Err(AppError::BadRequest {
                message: "Project name cannot be empty".to_string(),
            });
        }

        let project = ProjectRepo::create(
            db,
            tenant_id,
            req.name.trim(),
            req.description.as_deref(),
            req.task_type.as_deref(),
        )
        .await?;

        tracing::info!(
            project_id = %project.id,
            tenant_id = %tenant_id,
            "Project created"
        );

        Ok(project.into())
    }

    pub async fn get(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> AppResult<ProjectResponse> {
        let project = ProjectRepo::get_by_id(db, tenant_id, project_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Project not found".to_string(),
            })?;

        Ok(project.into())
    }

    pub async fn list(
        db: &PgPool,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<ProjectResponse>> {
        let (projects, total) = tokio::try_join!(
            ProjectRepo::list(db, tenant_id, offset, limit),
            ProjectRepo::count(db, tenant_id),
        )?;

        Ok(PaginatedResponse {
            data: projects.into_iter().map(Into::into).collect(),
            total,
            offset,
            limit,
        })
    }

    pub async fn update(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        req: UpdateProjectRequest,
    ) -> AppResult<ProjectResponse> {
        let project = ProjectRepo::update(
            db,
            tenant_id,
            project_id,
            req.name.as_deref(),
            req.description.as_deref(),
            req.task_type.as_deref(),
        )
        .await?
        .ok_or(AppError::NotFound {
            message: "Project not found".to_string(),
        })?;

        Ok(project.into())
    }

    pub async fn delete(db: &PgPool, tenant_id: Uuid, project_id: Uuid) -> AppResult<()> {
        let deleted = ProjectRepo::delete(db, tenant_id, project_id).await?;

        if !deleted {
            return Err(AppError::NotFound {
                message: "Project not found".to_string(),
            });
        }

        tracing::info!(
            project_id = %project_id,
            tenant_id = %tenant_id,
            "Project deleted"
        );

        Ok(())
    }
}
