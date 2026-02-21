use platform_db::models::TrainingJob;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Repository for training job database operations.
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct TrainingJobRepo;

impl TrainingJobRepo {
    /// Create a new training job.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        dataset_id: Uuid,
        base_model: &str,
        method: &str,
        mode: &str,
        hyperparams: serde_json::Value,
        gpu_class: Option<&str>,
        cost_estimate: Option<f64>,
    ) -> Result<TrainingJob, AppError> {
        let job = sqlx::query_as::<_, TrainingJob>(
            r#"
            INSERT INTO training_jobs
                (tenant_id, project_id, dataset_id, base_model, method, mode, hyperparams, gpu_class, cost_estimate)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(project_id)
        .bind(dataset_id)
        .bind(base_model)
        .bind(method)
        .bind(mode)
        .bind(hyperparams)
        .bind(gpu_class)
        .bind(cost_estimate)
        .fetch_one(db)
        .await?;

        Ok(job)
    }

    /// Get a single training job by ID.
    pub async fn get_by_id(
        db: &PgPool,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> Result<Option<TrainingJob>, AppError> {
        let job = sqlx::query_as::<_, TrainingJob>(
            "SELECT * FROM training_jobs WHERE id = $1 AND tenant_id = $2",
        )
        .bind(job_id)
        .bind(tenant_id)
        .fetch_optional(db)
        .await?;

        Ok(job)
    }

    /// List training jobs for a project within a tenant.
    pub async fn list_by_project(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<TrainingJob>, AppError> {
        let jobs = sqlx::query_as::<_, TrainingJob>(
            r#"
            SELECT * FROM training_jobs
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

        Ok(jobs)
    }

    /// Count training jobs for a project.
    pub async fn count_by_project(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM training_jobs WHERE project_id = $1 AND tenant_id = $2",
        )
        .bind(project_id)
        .bind(tenant_id)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Count training jobs by status for a project.
    pub async fn count_by_status(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        status: &str,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM training_jobs WHERE project_id = $1 AND tenant_id = $2 AND status = $3",
        )
        .bind(project_id)
        .bind(tenant_id)
        .bind(status)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Update the Temporal workflow ID after a workflow is started.
    pub async fn update_workflow_id(
        db: &PgPool,
        tenant_id: Uuid,
        job_id: Uuid,
        workflow_id: &str,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE training_jobs
            SET temporal_workflow_id = $3
            WHERE id = $1 AND tenant_id = $2
            "#,
        )
        .bind(job_id)
        .bind(tenant_id)
        .bind(workflow_id)
        .execute(db)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Cancel a training job (only if pending or cost_approval).
    pub async fn cancel(
        db: &PgPool,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> Result<Option<TrainingJob>, AppError> {
        let job = sqlx::query_as::<_, TrainingJob>(
            r#"
            UPDATE training_jobs
            SET status = 'cancelled'
            WHERE id = $1 AND tenant_id = $2 AND status IN ('pending', 'cost_approval')
            RETURNING *
            "#,
        )
        .bind(job_id)
        .bind(tenant_id)
        .fetch_optional(db)
        .await?;

        Ok(job)
    }
}
