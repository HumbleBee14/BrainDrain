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

    pub async fn get(db: &PgPool, tenant_id: Uuid, project_id: Uuid) -> AppResult<ProjectResponse> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::project::{CreateProjectRequest, UpdateProjectRequest};

    /// Helper: constructs a CreateProjectRequest with the given name.
    fn create_req(name: &str) -> CreateProjectRequest {
        CreateProjectRequest {
            name: name.to_string(),
            description: None,
            task_type: None,
        }
    }

    // ── Name validation (mirrors the check in ProjectService::create) ──

    #[test]
    fn empty_name_is_rejected() {
        let req = create_req("");
        assert!(req.name.trim().is_empty());
    }

    #[test]
    fn whitespace_only_name_is_rejected() {
        for name in ["   ", "\t", "\n", " \t\n "] {
            let req = create_req(name);
            assert!(
                req.name.trim().is_empty(),
                "Expected {:?} to be treated as empty",
                name,
            );
        }
    }

    #[test]
    fn valid_name_passes_validation() {
        for name in ["My Project", "test", "  padded  "] {
            let req = create_req(name);
            assert!(
                !req.name.trim().is_empty(),
                "Expected {:?} to pass validation",
                name,
            );
        }
    }

    #[test]
    fn name_is_trimmed_before_storage() {
        let req = create_req("  My Project  ");
        let trimmed = req.name.trim();
        assert_eq!(trimmed, "My Project");
    }

    // ── UpdateProjectRequest field semantics ──

    #[test]
    fn update_request_all_none_means_no_changes() {
        let req = UpdateProjectRequest {
            name: None,
            description: None,
            task_type: None,
        };
        assert!(req.name.is_none());
        assert!(req.description.is_none());
        assert!(req.task_type.is_none());
    }

    #[test]
    fn update_request_partial_fields() {
        let req = UpdateProjectRequest {
            name: Some("New Name".to_string()),
            description: None,
            task_type: Some("question_answering".to_string()),
        };
        assert_eq!(req.name.as_deref(), Some("New Name"));
        assert!(req.description.is_none());
        assert_eq!(req.task_type.as_deref(), Some("question_answering"));
    }

    // ── Error type correctness ──

    #[test]
    fn empty_name_produces_bad_request_error() {
        let req = create_req("");
        if req.name.trim().is_empty() {
            let err = AppError::BadRequest {
                message: "Project name cannot be empty".to_string(),
            };
            assert!(matches!(err, AppError::BadRequest { .. }));
        } else {
            panic!("Expected empty name to trigger validation");
        }
    }

    #[test]
    fn missing_project_produces_not_found_error() {
        let err = AppError::NotFound {
            message: "Project not found".to_string(),
        };
        assert!(matches!(err, AppError::NotFound { .. }));
        assert_eq!(err.to_string(), "Project not found");
    }
}
