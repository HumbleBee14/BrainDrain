use platform_shared::enums::DeploymentStatus;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dto::common::PaginatedResponse;
use crate::dto::model::ModelResponse;
use crate::error::{AppError, AppResult};
use crate::repositories::traits::ModelRepository;
use crate::services::deployment_service::DeploymentService;

/// Business logic for model operations.
pub struct ModelService;

impl ModelService {
    /// Get a single model.
    pub async fn get(
        repo: &dyn ModelRepository,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<ModelResponse> {
        let model = repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        Ok(model.into())
    }

    /// List models for a project.
    pub async fn list(
        repo: &dyn ModelRepository,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<ModelResponse>> {
        let (models, total) = tokio::try_join!(
            repo.list_by_project(tenant_id, project_id, offset, limit),
            repo.count_by_project(tenant_id, project_id),
        )?;

        Ok(PaginatedResponse {
            data: models.into_iter().map(Into::into).collect(),
            total,
            offset,
            limit,
        })
    }

    /// List all versions of a model (same base_model within a project).
    pub async fn list_versions(
        repo: &dyn ModelRepository,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<Vec<ModelResponse>> {
        let model = repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        let versions = repo
            .list_versions(tenant_id, model.project_id, &model.base_model)
            .await?;

        Ok(versions.into_iter().map(Into::into).collect())
    }

    /// Rollback: deploy a previous version and undeploy the currently active one.
    ///
    /// This re-points the serving backend for real (unload the current adapter,
    /// load the target's) via the same deploy/undeploy paths a normal deploy
    /// uses — a bare status flip in the database would leave the inference
    /// instance still serving the rolled-back-from model while the DB claimed
    /// the target was active. Both steps are idempotent (undeploy is skipped
    /// when the current version is already inactive, deploy when the target is
    /// already active), so a retried rollback converges instead of erroring.
    pub async fn rollback(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
        target_version_id: Uuid,
    ) -> AppResult<ModelResponse> {
        let repo = state.model_repo();

        let current = repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Current model not found".to_string(),
            })?;

        let target =
            repo.get_by_id(tenant_id, target_version_id)
                .await?
                .ok_or(AppError::NotFound {
                    message: "Target version not found".to_string(),
                })?;

        // Must be the same base_model within the same project
        if current.project_id != target.project_id || current.base_model != target.base_model {
            return Err(AppError::BadRequest {
                message: "Target version must be of the same base model in the same project"
                    .to_string(),
            });
        }

        if target.id == current.id {
            return Err(AppError::BadRequest {
                message: "Cannot rollback to the same version".to_string(),
            });
        }

        // Free the current adapter first so its slot is available before the
        // target claims one (keeps us under the per-base-model adapter cap).
        if current.deployment_status == DeploymentStatus::Active.to_string() {
            DeploymentService::undeploy(state, tenant_id, current.id).await?;
        }

        // Deploy the target for real; skip if it is already the active version.
        // Rollbacks bypass the eval gate: the target is a version that was
        // previously deployed (already vetted), and gating it could strand
        // production on a broken current version by refusing the restore.
        if target.deployment_status != DeploymentStatus::Active.to_string() {
            DeploymentService::deploy_bypassing_gate(state, tenant_id, target.id).await?;
        }

        let updated = repo
            .get_by_id(tenant_id, target.id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Target version not found after rollback".to_string(),
            })?;

        Ok(updated.into())
    }
}
