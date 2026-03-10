use platform_db::models::TrainingJob;
use platform_shared::enums::TrainingJobStatus;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, TrainingJobRepository};

/// PostgreSQL implementation of the training job repository.
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct PgTrainingJobRepo {
    db: PgPool,
}

impl PgTrainingJobRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl TrainingJobRepository for PgTrainingJobRepo {
    #[allow(clippy::too_many_arguments)]
    fn create(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        dataset_id: Uuid,
        base_model: &str,
        method: &str,
        mode: &str,
        hyperparams: serde_json::Value,
        gpu_class: Option<&str>,
        cost_estimate: Option<f64>,
    ) -> BoxFuture<'_, AppResult<TrainingJob>> {
        let base_model = base_model.to_string();
        let method = method.to_string();
        let mode = mode.to_string();
        let gpu_class = gpu_class.map(|s| s.to_string());
        Box::pin(async move {
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
            .bind(&base_model)
            .bind(&method)
            .bind(&mode)
            .bind(hyperparams)
            .bind(gpu_class.as_deref())
            .bind(cost_estimate)
            .fetch_one(&self.db)
            .await?;

            Ok(job)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn create_with_limit(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        dataset_id: Uuid,
        base_model: &str,
        method: &str,
        mode: &str,
        hyperparams: serde_json::Value,
        gpu_class: Option<&str>,
        cost_estimate: Option<f64>,
        max_models: i64,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        let base_model = base_model.to_string();
        let method = method.to_string();
        let mode = mode.to_string();
        let gpu_class = gpu_class.map(|s| s.to_string());
        Box::pin(async move {
            let job = sqlx::query_as::<_, TrainingJob>(
                r#"
                INSERT INTO training_jobs
                    (tenant_id, project_id, dataset_id, base_model, method, mode, hyperparams, gpu_class, cost_estimate)
                SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9
                WHERE (SELECT COUNT(*) FROM training_jobs WHERE tenant_id = $1) < $10
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(project_id)
            .bind(dataset_id)
            .bind(&base_model)
            .bind(&method)
            .bind(&mode)
            .bind(hyperparams)
            .bind(gpu_class.as_deref())
            .bind(cost_estimate)
            .bind(max_models)
            .fetch_optional(&self.db)
            .await?;

            Ok(job)
        })
    }

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        Box::pin(async move {
            let job = sqlx::query_as::<_, TrainingJob>(
                "SELECT * FROM training_jobs WHERE id = $1 AND tenant_id = $2",
            )
            .bind(job_id)
            .bind(tenant_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(job)
        })
    }

    fn list_by_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<TrainingJob>>> {
        Box::pin(async move {
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
            .fetch_all(&self.db)
            .await?;

            Ok(jobs)
        })
    }

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM training_jobs WHERE project_id = $1 AND tenant_id = $2",
            )
            .bind(project_id)
            .bind(tenant_id)
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }

    fn count_by_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: TrainingJobStatus,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM training_jobs WHERE project_id = $1 AND tenant_id = $2 AND status = $3",
            )
            .bind(project_id)
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }

    fn update_workflow_id(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
        workflow_id: &str,
    ) -> BoxFuture<'_, AppResult<bool>> {
        let workflow_id = workflow_id.to_string();
        Box::pin(async move {
            let result = sqlx::query(
                r#"
                UPDATE training_jobs
                SET temporal_workflow_id = $3
                WHERE id = $1 AND tenant_id = $2
                "#,
            )
            .bind(job_id)
            .bind(tenant_id)
            .bind(&workflow_id)
            .execute(&self.db)
            .await?;

            Ok(result.rows_affected() > 0)
        })
    }

    fn cancel(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        Box::pin(async move {
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
            .fetch_optional(&self.db)
            .await?;

            Ok(job)
        })
    }

    fn set_cost_approval(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        Box::pin(async move {
            let job = sqlx::query_as::<_, TrainingJob>(
                r#"
                UPDATE training_jobs
                SET status = 'cost_approval', updated_at = NOW()
                WHERE id = $1 AND tenant_id = $2 AND status = 'pending'
                RETURNING *
                "#,
            )
            .bind(job_id)
            .bind(tenant_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(job)
        })
    }

    fn approve_cost(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        Box::pin(async move {
            let job = sqlx::query_as::<_, TrainingJob>(
                r#"
                UPDATE training_jobs
                SET status = 'pending', updated_at = NOW()
                WHERE id = $1 AND tenant_id = $2 AND status = 'cost_approval'
                RETURNING *
                "#,
            )
            .bind(job_id)
            .bind(tenant_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(job)
        })
    }

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM training_jobs WHERE tenant_id = $1",
            )
            .bind(tenant_id)
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }

    fn count_by_tenant_status(
        &self,
        tenant_id: Uuid,
        status: TrainingJobStatus,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM training_jobs WHERE tenant_id = $1 AND status = $2",
            )
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }
}
