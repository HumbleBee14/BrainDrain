use platform_db::models::Evaluation;
use platform_db::tenant::begin_tenant_tx;
use platform_shared::enums::EvaluationStatus;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, EvaluationRepository};

/// PostgreSQL implementation of the evaluation repository.
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct PgEvaluationRepo {
    db: PgPool,
}

impl PgEvaluationRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl EvaluationRepository for PgEvaluationRepo {
    fn create(&self, tenant_id: Uuid, model_id: Uuid) -> BoxFuture<'_, AppResult<Evaluation>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let eval = sqlx::query_as::<_, Evaluation>(
                r#"
                INSERT INTO evaluations (tenant_id, model_id, status, started_at)
                VALUES ($1, $2, 'running', NOW())
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(model_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(eval)
        })
    }

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        eval_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Evaluation>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let eval = sqlx::query_as::<_, Evaluation>(
                "SELECT * FROM evaluations WHERE id = $1 AND tenant_id = $2",
            )
            .bind(eval_id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(eval)
        })
    }

    fn list_by_model(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Evaluation>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let evals = sqlx::query_as::<_, Evaluation>(
                r#"
                SELECT * FROM evaluations
                WHERE model_id = $1 AND tenant_id = $2
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(model_id)
            .bind(tenant_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(evals)
        })
    }

    fn count_by_model(&self, tenant_id: Uuid, model_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM evaluations WHERE model_id = $1 AND tenant_id = $2",
            )
            .bind(model_id)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*) FROM evaluations e
                JOIN models m ON m.id = e.model_id
                WHERE m.project_id = $1 AND e.tenant_id = $2
                "#,
            )
            .bind(project_id)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }

    fn count_by_project_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: EvaluationStatus,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*) FROM evaluations e
                JOIN models m ON m.id = e.model_id
                WHERE m.project_id = $1 AND e.tenant_id = $2 AND e.status = $3
                "#,
            )
            .bind(project_id)
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }

    fn update_workflow_id(
        &self,
        tenant_id: Uuid,
        eval_id: Uuid,
        workflow_id: &str,
    ) -> BoxFuture<'_, AppResult<bool>> {
        let workflow_id = workflow_id.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let result = sqlx::query(
                r#"
                UPDATE evaluations
                SET temporal_workflow_id = $3
                WHERE id = $1 AND tenant_id = $2
                "#,
            )
            .bind(eval_id)
            .bind(tenant_id)
            .bind(&workflow_id)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(result.rows_affected() > 0)
        })
    }

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM evaluations WHERE tenant_id = $1",
            )
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }
}
