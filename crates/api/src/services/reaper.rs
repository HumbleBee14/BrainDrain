//! Stuck-job reaper.
//!
//! A worker crash/OOM (or a terminated workflow whose cleanup never ran) can
//! leave a training job pinned in `training`/`provisioning` forever — GPU time
//! accrues but is never billed — or a document pinned in `parsing`. This
//! background pass finds those rows and forces a terminal state.
//!
//! Runs on the owner pool (RLS-exempt) so it sees every tenant's rows, and
//! bills reaped training jobs for elapsed GPU time in the same transaction as
//! the status change (durable, consistent with the worker's own billing).

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::services::billing_outbox::enqueue_in_tx;
use crate::services::training_job_service::{billable_gpu_cost, resolve_gpu_rate};
use crate::temporal::WorkflowOrchestrator;

/// A candidate stuck training job.
#[derive(sqlx::FromRow)]
struct StuckJob {
    id: Uuid,
    tenant_id: Uuid,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    gpu_class: Option<String>,
    temporal_workflow_id: Option<String>,
    mode: String,
    method: String,
    base_model: String,
}

/// Reap training jobs abandoned by a dead worker. Returns the number reaped.
///
/// A job past the stuck threshold is only reaped once we confirm its workflow
/// is no longer running (or is gone / was never recorded) — a legitimately long
/// run whose workflow is still executing is left alone.
pub async fn reap_stuck_training_jobs(
    db: &PgPool,
    orchestrator: Option<&dyn WorkflowOrchestrator>,
    stuck_after_secs: i64,
) -> Result<usize, sqlx::Error> {
    let candidates = sqlx::query_as::<_, StuckJob>(
        "SELECT id, tenant_id, started_at, gpu_class, temporal_workflow_id, \
                mode, method, base_model \
         FROM training_jobs \
         WHERE status IN ('training', 'provisioning') \
           AND updated_at < NOW() - make_interval(secs => $1)",
    )
    .bind(stuck_after_secs as f64)
    .fetch_all(db)
    .await?;

    let mut reaped = 0;
    for job in candidates {
        if workflow_still_running(orchestrator, job.temporal_workflow_id.as_deref()).await {
            continue;
        }
        if reap_one_training_job(db, &job).await? {
            reaped += 1;
        }
    }
    Ok(reaped)
}

/// True if the job's workflow is confirmed still running. On a transient
/// orchestrator error we return `true` (skip reaping this cycle) rather than
/// risk killing a live job; a missing workflow id means nothing is running.
async fn workflow_still_running(
    orchestrator: Option<&dyn WorkflowOrchestrator>,
    workflow_id: Option<&str>,
) -> bool {
    let (Some(orch), Some(wf)) = (orchestrator, workflow_id) else {
        return false;
    };
    match orch.get_workflow_status(wf).await {
        Ok(status) => status_indicates_running(&status.status),
        Err(crate::temporal::OrchestratorError::Api { status: 404, .. }) => false,
        Err(_) => true,
    }
}

/// A Temporal execution status string denotes a still-running workflow.
/// The HTTP API reports either `Running` or `WORKFLOW_EXECUTION_STATUS_RUNNING`;
/// every closed state (Completed/Failed/Terminated/TimedOut/Canceled) is not.
fn status_indicates_running(status: &str) -> bool {
    status.to_uppercase().contains("RUNNING")
}

/// Mark one job failed and bill the GPU time used, transactionally. Returns
/// whether a row was reaped (false if it already left the running state).
async fn reap_one_training_job(db: &PgPool, job: &StuckJob) -> Result<bool, sqlx::Error> {
    let elapsed = job
        .started_at
        .map(|s| (chrono::Utc::now() - s).num_seconds().max(0))
        .unwrap_or(0);
    let rate = resolve_gpu_rate(&tenant_gpu_rates(db, job.tenant_id).await?, &gpu_class(job));
    let (gpu_seconds, cost) = billable_gpu_cost(elapsed, rate);

    let mut tx = db.begin().await?;

    let updated = sqlx::query(
        "UPDATE training_jobs \
         SET status = 'failed', \
             error_message = 'Training worker stopped responding; job reaped', \
             actual_cost = $2, completed_at = NOW() \
         WHERE id = $1 AND status IN ('training', 'provisioning')",
    )
    .bind(job.id)
    .bind(cost)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    enqueue_in_tx(
        &mut tx,
        job.tenant_id,
        "training",
        Some(job.id),
        0,
        0,
        gpu_seconds,
        cost,
        serde_json::json!({
            "status": "failed",
            "reaped": true,
            "mode": job.mode,
            "method": job.method,
            "base_model": job.base_model,
            "gpu_class": job.gpu_class,
        }),
    )
    .await?;

    tx.commit().await?;
    tracing::warn!(training_job_id = %job.id, gpu_seconds, cost, "Reaped stuck training job");
    Ok(true)
}

/// The tenant's configured GPU rates, read directly from the owner pool.
/// Empty when unset — `resolve_gpu_rate` then falls back to the list rates.
async fn tenant_gpu_rates(
    db: &PgPool,
    tenant_id: Uuid,
) -> Result<HashMap<String, f64>, sqlx::Error> {
    let rates: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT settings->'admin'->'gpu_rates' FROM tenants WHERE id = $1")
            .bind(tenant_id)
            .fetch_optional(db)
            .await?
            .flatten();

    Ok(rates
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

fn gpu_class(job: &StuckJob) -> String {
    job.gpu_class.as_deref().unwrap_or("").to_lowercase()
}

/// Fail documents stuck in `parsing` past the threshold. Returns the count.
pub async fn reap_stuck_parsing_documents(
    db: &PgPool,
    stuck_after_secs: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE documents \
         SET status = 'failed', \
             error_message = 'Parsing worker stopped responding; document reaped', \
             updated_at = NOW() \
         WHERE status = 'parsing' \
           AND updated_at < NOW() - make_interval(secs => $1)",
    )
    .bind(stuck_after_secs as f64)
    .execute(db)
    .await?;

    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::status_indicates_running;

    #[test]
    fn running_statuses_detected() {
        assert!(status_indicates_running("Running"));
        assert!(status_indicates_running(
            "WORKFLOW_EXECUTION_STATUS_RUNNING"
        ));
    }

    #[test]
    fn closed_statuses_not_running() {
        for s in [
            "Completed",
            "Failed",
            "Terminated",
            "TimedOut",
            "Canceled",
            "WORKFLOW_EXECUTION_STATUS_TERMINATED",
            "UNKNOWN",
        ] {
            assert!(!status_indicates_running(s), "{s} should not be running");
        }
    }
}
