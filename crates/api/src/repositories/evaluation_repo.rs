use platform_db::models::Evaluation;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Repository for evaluation database operations.
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct EvaluationRepo;

impl EvaluationRepo {
    /// Create a new evaluation record.
    pub async fn create(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> Result<Evaluation, AppError> {
        let eval = sqlx::query_as::<_, Evaluation>(
            r#"
            INSERT INTO evaluations (tenant_id, model_id, status, started_at)
            VALUES ($1, $2, 'running', NOW())
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(model_id)
        .fetch_one(db)
        .await?;

        Ok(eval)
    }

    /// Get a single evaluation by ID.
    pub async fn get_by_id(
        db: &PgPool,
        tenant_id: Uuid,
        eval_id: Uuid,
    ) -> Result<Option<Evaluation>, AppError> {
        let eval = sqlx::query_as::<_, Evaluation>(
            "SELECT * FROM evaluations WHERE id = $1 AND tenant_id = $2",
        )
        .bind(eval_id)
        .bind(tenant_id)
        .fetch_optional(db)
        .await?;

        Ok(eval)
    }

    /// List evaluations for a model within a tenant.
    pub async fn list_by_model(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<Evaluation>, AppError> {
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
        .fetch_all(db)
        .await?;

        Ok(evals)
    }

    /// Count evaluations for a model.
    pub async fn count_by_model(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM evaluations WHERE model_id = $1 AND tenant_id = $2",
        )
        .bind(model_id)
        .bind(tenant_id)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Count evaluations for a project (across all models).
    pub async fn count_by_project(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*) FROM evaluations e
            JOIN models m ON m.id = e.model_id
            WHERE m.project_id = $1 AND e.tenant_id = $2
            "#,
        )
        .bind(project_id)
        .bind(tenant_id)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Count evaluations by status for a project.
    pub async fn count_by_project_status(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        status: &str,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*) FROM evaluations e
            JOIN models m ON m.id = e.model_id
            WHERE m.project_id = $1 AND e.tenant_id = $2 AND e.status = $3
            "#,
        )
        .bind(project_id)
        .bind(tenant_id)
        .bind(status)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Update the Temporal workflow ID for an evaluation.
    pub async fn update_workflow_id(
        db: &PgPool,
        tenant_id: Uuid,
        eval_id: Uuid,
        workflow_id: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE evaluations
            SET temporal_workflow_id = $3
            WHERE id = $1 AND tenant_id = $2
            "#,
        )
        .bind(eval_id)
        .bind(tenant_id)
        .bind(workflow_id)
        .execute(db)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
