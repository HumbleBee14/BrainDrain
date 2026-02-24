use platform_shared::enums::DeploymentStatus;
use uuid::Uuid;

use crate::dto::common::PaginatedResponse;
use crate::dto::model::ModelResponse;
use crate::error::{AppError, AppResult};
use crate::repositories::traits::ModelRepository;

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
    pub async fn rollback(
        repo: &dyn ModelRepository,
        tenant_id: Uuid,
        model_id: Uuid,
        target_version_id: Uuid,
    ) -> AppResult<ModelResponse> {
        let current = repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Current model not found".to_string(),
            })?;

        let target = repo
            .get_by_id(tenant_id, target_version_id)
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

        // Undeploy current if active, deploy target
        if current.deployment_status == "active" {
            repo.update_deployment_status(tenant_id, current.id, DeploymentStatus::Undeployed)
                .await?;
        }

        let updated = repo
            .update_deployment_status(tenant_id, target.id, DeploymentStatus::Active)
            .await?
            .ok_or(AppError::NotFound {
                message: "Failed to update target deployment status".to_string(),
            })?;

        Ok(updated.into())
    }
}
