use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::common::PaginatedResponse;
use crate::dto::model::ModelResponse;
use crate::error::{AppError, AppResult};
use crate::repositories::model_repo::ModelRepo;

/// Business logic for model operations (read-only from API side).
pub struct ModelService;

impl ModelService {
    /// Get a single model.
    pub async fn get(db: &PgPool, tenant_id: Uuid, model_id: Uuid) -> AppResult<ModelResponse> {
        let model =
            ModelRepo::get_by_id(db, tenant_id, model_id)
                .await?
                .ok_or(AppError::NotFound {
                    message: "Model not found".to_string(),
                })?;

        Ok(model.into())
    }

    /// List models for a project.
    pub async fn list(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<ModelResponse>> {
        let (models, total) = tokio::try_join!(
            ModelRepo::list_by_project(db, tenant_id, project_id, offset, limit),
            ModelRepo::count_by_project(db, tenant_id, project_id),
        )?;

        Ok(PaginatedResponse {
            data: models.into_iter().map(Into::into).collect(),
            total,
            offset,
            limit,
        })
    }
}
