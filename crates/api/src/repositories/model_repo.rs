use platform_db::models::Model;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Repository for model database operations (read-only from API side).
///
/// Models are created by the Python training worker.
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct ModelRepo;

impl ModelRepo {
    /// Get a single model by ID.
    pub async fn get_by_id(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> Result<Option<Model>, AppError> {
        let model =
            sqlx::query_as::<_, Model>("SELECT * FROM models WHERE id = $1 AND tenant_id = $2")
                .bind(model_id)
                .bind(tenant_id)
                .fetch_optional(db)
                .await?;

        Ok(model)
    }

    /// List models for a project within a tenant.
    pub async fn list_by_project(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<Model>, AppError> {
        let models = sqlx::query_as::<_, Model>(
            r#"
            SELECT * FROM models
            WHERE project_id = $1 AND tenant_id = $2
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(project_id)
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(db)
        .await?;

        Ok(models)
    }

    /// Count models for a project.
    pub async fn count_by_project(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM models WHERE project_id = $1 AND tenant_id = $2",
        )
        .bind(project_id)
        .bind(tenant_id)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Count models by deployment status for a project.
    pub async fn count_by_deployment_status(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        status: &str,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM models WHERE project_id = $1 AND tenant_id = $2 AND deployment_status = $3",
        )
        .bind(project_id)
        .bind(tenant_id)
        .bind(status)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Update a model's deployment status.
    pub async fn update_deployment_status(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
        status: &str,
    ) -> Result<Option<Model>, AppError> {
        let model = sqlx::query_as::<_, Model>(
            r#"
            UPDATE models
            SET deployment_status = $3, updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(model_id)
        .bind(tenant_id)
        .bind(status)
        .fetch_optional(db)
        .await?;

        Ok(model)
    }

    /// Update deployment status and config together.
    pub async fn update_deployment(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
        status: &str,
        config: serde_json::Value,
    ) -> Result<Option<Model>, AppError> {
        let model = sqlx::query_as::<_, Model>(
            r#"
            UPDATE models
            SET deployment_status = $3, deployment_config = $4, updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(model_id)
        .bind(tenant_id)
        .bind(status)
        .bind(config)
        .fetch_optional(db)
        .await?;

        Ok(model)
    }

    /// Update a model's eval_scores.
    #[allow(dead_code)]
    pub async fn update_eval_scores(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
        scores: serde_json::Value,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE models
            SET eval_scores = $3, updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            "#,
        )
        .bind(model_id)
        .bind(tenant_id)
        .bind(scores)
        .execute(db)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
