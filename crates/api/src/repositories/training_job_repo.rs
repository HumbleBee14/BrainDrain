use platform_db::models::TrainingJob;
use platform_db::tenant::begin_tenant_tx;
use platform_shared::enums::TrainingJobStatus;
use sqlx::PgPool;
use sqlx::Postgres;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::billing_event_repo::DELIVERED_AND_IN_FLIGHT_COST_SQL;
use crate::repositories::traits::{BoxFuture, TrainingJobRepository};
use crate::services::teacher::billing::TeacherSpendReservation;
use crate::services::teacher::serving_cost::teacher_reservation_billing_event_id;

/// The outcome a user-cancelled run is billed under. Distinct from the worker's
/// own outcomes because only one side ever wins the terminal status transition.
const CANCELLED_OUTCOME: &str = "cancelled";

/// Re-check the teacher-GPU budget and write the run's reservation, inside the
/// job-creation transaction.
///
/// The admission check the service ran before this read the budget without a
/// lock: two concurrent admissions can both pass it and together land over the
/// cap. This one takes a per-tenant advisory lock first, so between its read
/// and its write no other admission can do either — the second one blocks, then
/// reads a budget the first has already joined.
async fn check_cap_and_reserve(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    tenant_id: Uuid,
    job_id: Uuid,
    reservation: TeacherSpendReservation,
) -> AppResult<()> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('teacher_gpu_spend:' || $1::text, 0))",
    )
    .bind(tenant_id)
    .execute(&mut **tx)
    .await?;

    let spent = sqlx::query_scalar::<_, f64>(DELIVERED_AND_IN_FLIGHT_COST_SQL)
        .bind(tenant_id)
        .bind(&reservation.counted_operations)
        .bind(reservation.month_start)
        .fetch_one(&mut **tx)
        .await?;

    reservation.admit_against(spent)?;

    sqlx::query(
        "INSERT INTO billing_outbox \
            (id, tenant_id, operation, resource_id, tokens_in, tokens_out, \
             gpu_seconds, cost_usd, metadata) \
         VALUES ($1, $2, 'teacher_serving', $3, 0, 0, $4, $5, $6) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(teacher_reservation_billing_event_id(job_id))
    .bind(tenant_id)
    .bind(job_id)
    .bind(reservation.gpu_seconds)
    .bind(reservation.cost_usd)
    .bind(reservation.metadata)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

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
        teacher_config: Option<serde_json::Value>,
        parent_model_id: Option<Uuid>,
        reservation: Option<TeacherSpendReservation>,
    ) -> BoxFuture<'_, AppResult<TrainingJob>> {
        let base_model = base_model.to_string();
        let method = method.to_string();
        let mode = mode.to_string();
        let gpu_class = gpu_class.map(|s| s.to_string());
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let job = sqlx::query_as::<_, TrainingJob>(
                r#"
                INSERT INTO training_jobs
                    (tenant_id, project_id, dataset_id, base_model, method, mode, hyperparams, gpu_class, cost_estimate, teacher_config, parent_model_id)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
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
            .bind(teacher_config)
            .bind(parent_model_id)
            .fetch_one(&mut *tx)
            .await?;

            if let Some(reservation) = reservation {
                check_cap_and_reserve(&mut tx, tenant_id, job.id, reservation).await?;
            }

            tx.commit().await?;
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
        teacher_config: Option<serde_json::Value>,
        parent_model_id: Option<Uuid>,
        max_models: i64,
        reservation: Option<TeacherSpendReservation>,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        let base_model = base_model.to_string();
        let method = method.to_string();
        let mode = mode.to_string();
        let gpu_class = gpu_class.map(|s| s.to_string());
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let job = sqlx::query_as::<_, TrainingJob>(
                r#"
                INSERT INTO training_jobs
                    (tenant_id, project_id, dataset_id, base_model, method, mode, hyperparams, gpu_class, cost_estimate, teacher_config, parent_model_id)
                SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11
                WHERE (
                    SELECT COUNT(*) FROM training_jobs
                    WHERE tenant_id = $1 AND status NOT IN ('failed', 'cancelled')
                ) < $12
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
            .bind(teacher_config)
            .bind(parent_model_id)
            .bind(max_models)
            .fetch_optional(&mut *tx)
            .await?;

            if let (Some(job), Some(reservation)) = (&job, reservation) {
                check_cap_and_reserve(&mut tx, tenant_id, job.id, reservation).await?;
            }

            tx.commit().await?;
            Ok(job)
        })
    }

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let job = sqlx::query_as::<_, TrainingJob>(
                "SELECT * FROM training_jobs WHERE id = $1 AND tenant_id = $2",
            )
            .bind(job_id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
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
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
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
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(jobs)
        })
    }

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM training_jobs WHERE project_id = $1 AND tenant_id = $2",
            )
            .bind(project_id)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
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
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM training_jobs WHERE project_id = $1 AND tenant_id = $2 AND status = $3",
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
        job_id: Uuid,
        workflow_id: &str,
    ) -> BoxFuture<'_, AppResult<bool>> {
        let workflow_id = workflow_id.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
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
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(result.rows_affected() > 0)
        })
    }

    fn cancel(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
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
            .fetch_optional(&mut *tx)
            .await?;

            // A run cancelled before it started never held a GPU, so its
            // admission reservation goes with it. The stale reaper would delete
            // it too — two days from now; releasing it here frees the tenant's
            // cap headroom the moment they cancel.
            if job.is_some() {
                sqlx::query(
                    "DELETE FROM billing_outbox \
                     WHERE id = $1 AND tenant_id = $2 AND delivered_at IS NULL",
                )
                .bind(teacher_reservation_billing_event_id(job_id))
                .bind(tenant_id)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(job)
        })
    }

    fn finalize_cancelled(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
        actual_cost: f64,
        gpu_seconds: i32,
        metadata: serde_json::Value,
        teacher_share: f64,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;

            let job = sqlx::query_as::<_, TrainingJob>(
                r#"
                UPDATE training_jobs
                SET status = 'cancelled', actual_cost = $3, completed_at = NOW()
                WHERE id = $1 AND tenant_id = $2 AND status IN ('training', 'provisioning')
                RETURNING *
                "#,
            )
            .bind(job_id)
            .bind(tenant_id)
            .bind(actual_cost)
            .fetch_optional(&mut *tx)
            .await?;

            // Only bill when the state transition actually happened (idempotent
            // against repeated cancels).
            if job.is_some() {
                crate::services::teacher::serving_cost::enqueue_run_billing(
                    &mut tx,
                    tenant_id,
                    job_id,
                    CANCELLED_OUTCOME,
                    crate::services::teacher::serving_cost::RunCharge {
                        gpu_seconds,
                        cost_usd: actual_cost,
                        metadata,
                        teacher_share,
                    },
                )
                .await?;
            }

            tx.commit().await?;
            Ok(job)
        })
    }

    fn set_cost_approval(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
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
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(job)
        })
    }

    fn approve_cost(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
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
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(job)
        })
    }

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM training_jobs WHERE tenant_id = $1",
            )
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }

    fn count_by_tenant_status(
        &self,
        tenant_id: Uuid,
        status: TrainingJobStatus,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM training_jobs WHERE tenant_id = $1 AND status = $2",
            )
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }
}
