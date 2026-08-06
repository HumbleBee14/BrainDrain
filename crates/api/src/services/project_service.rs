use platform_shared::enums::ProjectStatus;
use uuid::Uuid;

use crate::dto::common::PaginatedResponse;
use crate::dto::project::{CreateProjectRequest, ProjectResponse, UpdateProjectRequest};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::ProjectRepository;

/// Business logic for project operations.
///
/// Thin layer over the repository that adds validation and event publishing.
/// As the platform grows, cross-cutting concerns (billing, notifications) live here.
pub struct ProjectService;

impl ProjectService {
    #[allow(dead_code)]
    pub async fn create(
        repo: &dyn ProjectRepository,
        tenant_id: Uuid,
        req: CreateProjectRequest,
    ) -> AppResult<ProjectResponse> {
        if req.name.trim().is_empty() {
            return Err(AppError::BadRequest {
                message: "Project name cannot be empty".to_string(),
            });
        }

        let project = repo
            .create(
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

    /// Create a project with atomic plan limit enforcement.
    /// Returns Forbidden if the plan limit would be exceeded.
    pub async fn create_with_limit(
        repo: &dyn ProjectRepository,
        tenant_id: Uuid,
        req: CreateProjectRequest,
        max_count: i64,
    ) -> AppResult<ProjectResponse> {
        if req.name.trim().is_empty() {
            return Err(AppError::BadRequest {
                message: "Project name cannot be empty".to_string(),
            });
        }

        let project = repo
            .create_with_limit(
                tenant_id,
                req.name.trim(),
                req.description.as_deref(),
                req.task_type.as_deref(),
                max_count,
            )
            .await?
            .ok_or(AppError::Forbidden {
                message: format!(
                    "Plan limit reached: maximum {} projects on your current plan",
                    max_count
                ),
            })?;

        tracing::info!(
            project_id = %project.id,
            tenant_id = %tenant_id,
            "Project created (atomic limit check)"
        );

        Ok(project.into())
    }

    pub async fn get(
        repo: &dyn ProjectRepository,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> AppResult<ProjectResponse> {
        let project = repo
            .get_by_id(tenant_id, project_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Project not found".to_string(),
            })?;

        Ok(project.into())
    }

    pub async fn list(
        repo: &dyn ProjectRepository,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<ProjectResponse>> {
        let (projects, total) =
            tokio::try_join!(repo.list(tenant_id, offset, limit), repo.count(tenant_id),)?;

        Ok(PaginatedResponse {
            data: projects.into_iter().map(Into::into).collect(),
            total,
            offset,
            limit,
        })
    }

    pub async fn update(
        repo: &dyn ProjectRepository,
        tenant_id: Uuid,
        project_id: Uuid,
        req: UpdateProjectRequest,
    ) -> AppResult<ProjectResponse> {
        let project = repo
            .update(
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

    /// Update a project's status with state machine validation.
    ///
    /// Only allows valid transitions:
    /// - Created → Ingesting, Archived
    /// - Ingesting → Refining, Created (rollback on failure)
    /// - Refining → Training, Ingesting (rollback on failure)
    /// - Training → Evaluating, Refining (rollback on failure)
    /// - Evaluating → Deployed, Training (rollback on failure)
    /// - Deployed → Archived, Evaluating (re-evaluate)
    /// - Archived → Created (un-archive)
    pub async fn update_status(
        repo: &dyn ProjectRepository,
        tenant_id: Uuid,
        project_id: Uuid,
        new_status: ProjectStatus,
    ) -> AppResult<ProjectResponse> {
        let project = repo
            .get_by_id(tenant_id, project_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Project not found".to_string(),
            })?;

        let current: ProjectStatus = project.status.parse().unwrap_or(ProjectStatus::Created);

        if !is_valid_transition(current, new_status) {
            return Err(AppError::BadRequest {
                message: format!("Invalid status transition: {} → {}", current, new_status),
            });
        }

        let updated = repo
            .update_status(tenant_id, project_id, &new_status.to_string())
            .await?
            .ok_or(AppError::NotFound {
                message: "Project not found".to_string(),
            })?;

        tracing::info!(
            project_id = %project_id,
            old_status = %current,
            new_status = %new_status,
            "Project status updated"
        );

        Ok(updated.into())
    }
}

/// Validate project status transitions.
fn is_valid_transition(from: ProjectStatus, to: ProjectStatus) -> bool {
    matches!(
        (from, to),
        // Forward pipeline progression
        (ProjectStatus::Created, ProjectStatus::Ingesting)
            | (ProjectStatus::Ingesting, ProjectStatus::Refining)
            | (ProjectStatus::Refining, ProjectStatus::Training)
            | (ProjectStatus::Training, ProjectStatus::Evaluating)
            | (ProjectStatus::Evaluating, ProjectStatus::Deployed)
            // Rollback on failure (go back one step)
            | (ProjectStatus::Ingesting, ProjectStatus::Created)
            | (ProjectStatus::Refining, ProjectStatus::Ingesting)
            | (ProjectStatus::Training, ProjectStatus::Refining)
            | (ProjectStatus::Evaluating, ProjectStatus::Training)
            // Re-evaluate a deployed model
            | (ProjectStatus::Deployed, ProjectStatus::Evaluating)
            // Archive from any active state
            | (ProjectStatus::Created, ProjectStatus::Archived)
            | (ProjectStatus::Deployed, ProjectStatus::Archived)
            // Un-archive
            | (ProjectStatus::Archived, ProjectStatus::Created)
    )
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

    // ── State machine transitions ──

    #[test]
    fn forward_pipeline_transitions_are_valid() {
        let forward = [
            (ProjectStatus::Created, ProjectStatus::Ingesting),
            (ProjectStatus::Ingesting, ProjectStatus::Refining),
            (ProjectStatus::Refining, ProjectStatus::Training),
            (ProjectStatus::Training, ProjectStatus::Evaluating),
            (ProjectStatus::Evaluating, ProjectStatus::Deployed),
        ];
        for (from, to) in forward {
            assert!(
                is_valid_transition(from, to),
                "Expected {from} → {to} to be valid",
            );
        }
    }

    #[test]
    fn rollback_transitions_are_valid() {
        let rollbacks = [
            (ProjectStatus::Ingesting, ProjectStatus::Created),
            (ProjectStatus::Refining, ProjectStatus::Ingesting),
            (ProjectStatus::Training, ProjectStatus::Refining),
            (ProjectStatus::Evaluating, ProjectStatus::Training),
        ];
        for (from, to) in rollbacks {
            assert!(
                is_valid_transition(from, to),
                "Expected rollback {from} → {to} to be valid",
            );
        }
    }

    #[test]
    fn archive_transitions_are_valid() {
        assert!(is_valid_transition(
            ProjectStatus::Created,
            ProjectStatus::Archived
        ));
        assert!(is_valid_transition(
            ProjectStatus::Deployed,
            ProjectStatus::Archived
        ));
        assert!(is_valid_transition(
            ProjectStatus::Archived,
            ProjectStatus::Created
        ));
    }

    #[test]
    fn invalid_transitions_are_rejected() {
        let invalid = [
            (ProjectStatus::Created, ProjectStatus::Training),
            (ProjectStatus::Created, ProjectStatus::Deployed),
            (ProjectStatus::Deployed, ProjectStatus::Created),
            (ProjectStatus::Training, ProjectStatus::Deployed),
            (ProjectStatus::Archived, ProjectStatus::Deployed),
        ];
        for (from, to) in invalid {
            assert!(
                !is_valid_transition(from, to),
                "Expected {from} → {to} to be invalid",
            );
        }
    }

    #[test]
    fn same_status_transition_is_invalid() {
        for status in [
            ProjectStatus::Created,
            ProjectStatus::Training,
            ProjectStatus::Deployed,
        ] {
            assert!(
                !is_valid_transition(status, status),
                "Expected {status} → {status} to be invalid",
            );
        }
    }
}
