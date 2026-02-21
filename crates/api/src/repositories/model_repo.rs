use platform_db::models::Model;
use platform_shared::enums::DeploymentStatus;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, ModelRepository};

/// PostgreSQL implementation of the model repository.
///
/// Models are created by the Python training worker.
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct PgModelRepo {
    db: PgPool,
}

impl PgModelRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl ModelRepository for PgModelRepo {
    fn get_by_id(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Model>>> {
        Box::pin(async move {
            let model =
                sqlx::query_as::<_, Model>("SELECT * FROM models WHERE id = $1 AND tenant_id = $2")
                    .bind(model_id)
                    .bind(tenant_id)
                    .fetch_optional(&self.db)
                    .await?;

            Ok(model)
        })
    }

    fn list_by_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Model>>> {
        Box::pin(async move {
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
            .fetch_all(&self.db)
            .await?;

            Ok(models)
        })
    }

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM models WHERE project_id = $1 AND tenant_id = $2",
            )
            .bind(project_id)
            .bind(tenant_id)
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }

    fn count_by_deployment_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: DeploymentStatus,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM models WHERE project_id = $1 AND tenant_id = $2 AND deployment_status = $3",
            )
            .bind(project_id)
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }

    fn update_deployment_status(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        status: DeploymentStatus,
    ) -> BoxFuture<'_, AppResult<Option<Model>>> {
        Box::pin(async move {
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
            .bind(status.to_string())
            .fetch_optional(&self.db)
            .await?;

            Ok(model)
        })
    }

    fn update_deployment(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        status: DeploymentStatus,
        config: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<Option<Model>>> {
        Box::pin(async move {
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
            .bind(status.to_string())
            .bind(config)
            .fetch_optional(&self.db)
            .await?;

            Ok(model)
        })
    }

    fn update_eval_scores(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        scores: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<bool>> {
        Box::pin(async move {
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
            .execute(&self.db)
            .await?;

            Ok(result.rows_affected() > 0)
        })
    }
}
