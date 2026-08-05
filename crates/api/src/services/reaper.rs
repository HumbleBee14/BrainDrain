//! Background reapers for abandoned/idle resources.
//!
//! A worker crash/OOM (or a terminated workflow whose cleanup never ran) can
//! leave a training job pinned in `training`/`provisioning` forever — GPU time
//! accrues but is never billed — or pinned in `pending` if it died while a
//! teacher was still scoring for it, or a document pinned in `parsing`. A deploy
//! request that dies mid-flight likewise leaves a model pinned in `deploying`,
//! holding an inference-instance slot and blocking redeploys. Idle serving
//! instances similarly keep an external GPU box running with no traffic. These
//! background passes find those rows and force a terminal/scaled-down state.
//!
//! Runs on the owner pool (RLS-exempt) so it sees every tenant's rows, and
//! bills reaped training jobs for elapsed GPU time in the same transaction as
//! the status change (durable, consistent with the worker's own billing).
//!
//! Idle-instance reaping scales the control-plane state to zero (undeploy the
//! instance's models, retire the instance). Tearing down the underlying GPU box
//! remains the operator's/provider's responsibility — instances are registered
//! externally, so the reaper cannot recreate one it destroys.
//!
//! The orphaned-object sweep reclaims object storage: a document that ends in
//! `failed` keeps its uploaded source object forever otherwise, since parsing
//! never reads a failed source again. After a grace period the object is
//! deleted and the row's `storage_path` cleared.

use std::collections::HashMap;

use platform_storage::{ObjectStorage, StorageError};
use sqlx::PgPool;
use uuid::Uuid;

use crate::services::teacher::serving_cost::{
    RunCharge, enqueue_run_billing, teacher_serving_share,
};
use crate::services::training_job_service::{billable_gpu_cost, resolve_gpu_rate};
use crate::temporal::WorkflowOrchestrator;

/// The outcome a reaped run is billed under. Matches the worker's own name for a
/// failed run so the two writes address one ledger row, not two.
const REAPED_OUTCOME: &str = "failed";

/// A candidate stuck training job.
#[derive(sqlx::FromRow)]
struct StuckJob {
    id: Uuid,
    tenant_id: Uuid,
    status: String,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    gpu_class: Option<String>,
    temporal_workflow_id: Option<String>,
    mode: String,
    method: String,
    base_model: String,
    teacher_extraction_status: Option<String>,
    hyperparams: serde_json::Value,
}

/// Jobs abandoned by a dead worker, in any of the states one can be abandoned
/// in: mid-training, or still `pending` because a scoring pass ran first. The
/// second arm covers both a scoring pass that died with its worker and a
/// workflow terminated after scoring finished but before training claimed the
/// job — either way the row would otherwise sit untouched forever and the
/// teacher's GPU call unswept. A `pending` job that never had a scoring pass
/// carries a NULL extraction status and stays out of reach.
const STUCK_JOB_PREDICATE: &str = "(status IN ('training', 'provisioning') \
     OR (status = 'pending' AND teacher_extraction_status IN ('running', 'completed')))";

fn stuck_job_select_sql() -> String {
    format!(
        "SELECT id, tenant_id, status, started_at, gpu_class, temporal_workflow_id, \
                mode, method, base_model, teacher_extraction_status, hyperparams \
         FROM training_jobs \
         WHERE {STUCK_JOB_PREDICATE} \
           AND updated_at < NOW() - make_interval(secs => $1)"
    )
}

/// Shares `STUCK_JOB_PREDICATE` with the select so the two cannot disagree about
/// what is stuck: a row the select claims and the update declines would be
/// re-read and skipped on every pass, forever.
fn reap_update_sql() -> String {
    format!(
        "UPDATE training_jobs \
         SET status = 'failed', error_message = $3, \
             actual_cost = $2, completed_at = NOW() \
         WHERE id = $1 AND {STUCK_JOB_PREDICATE}"
    )
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
    let candidates = sqlx::query_as::<_, StuckJob>(&stuck_job_select_sql())
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

/// What the tenant is told about a job the reaper closed. A job abandoned before
/// training claimed it never ran a step, so blaming the trainer would send them
/// looking in the wrong place — and if the scoring pass finished, blaming the
/// scoring pass would too. Only a still-`pending` job can be pre-training;
/// once training claims it, a `completed` extraction status just records history.
fn reaped_message(job: &StuckJob) -> &'static str {
    if job.status != "pending" {
        return "Training worker stopped responding; job reaped";
    }
    if job.teacher_extraction_status.as_deref() == Some("completed") {
        "The worker stopped responding after the teacher finished scoring; \
         job reaped before training started"
    } else {
        "The teacher scoring pass stopped responding; job reaped before training started"
    }
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

    let updated = sqlx::query(&reap_update_sql())
        .bind(job.id)
        .bind(cost)
        .bind(reaped_message(job))
        .execute(&mut *tx)
        .await?;

    if updated.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    // Split so the teacher-GPU cap can see an improve pass the worker never got
    // to close out. Billed under the same ledger ids the worker would have used,
    // so a late write from a worker that outlived its reaping is a no-op.
    enqueue_run_billing(
        &mut tx,
        job.tenant_id,
        job.id,
        REAPED_OUTCOME,
        RunCharge {
            gpu_seconds,
            cost_usd: cost,
            metadata: serde_json::json!({
                "status": "failed",
                "reaped": true,
                "mode": job.mode,
                "method": job.method,
                "base_model": job.base_model,
                "gpu_class": job.gpu_class,
            }),
            teacher_share: teacher_serving_share(job.gpu_class.as_deref(), &job.hyperparams),
        },
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

/// Whether idle-instance reaping is enabled (a positive timeout).
fn idle_reaping_enabled(idle_after_secs: i64) -> bool {
    idle_after_secs > 0
}

/// Scale idle serving instances to zero. An instance is idle when none of its
/// deployed models has served inference (via any API key) within the timeout —
/// falling back to the instance's creation time when it has never been used.
/// Such an instance has its models undeployed and is retired, freeing the
/// externally-managed GPU box for teardown. Disabled when `idle_after_secs <= 0`.
/// Returns the number of instances retired.
pub async fn reap_idle_instances(db: &PgPool, idle_after_secs: i64) -> Result<usize, sqlx::Error> {
    if !idle_reaping_enabled(idle_after_secs) {
        return Ok(0);
    }

    let idle_instances: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT i.id, i.name \
         FROM inference_instances i \
         WHERE i.lifecycle_state = 'ready' \
           AND COALESCE( \
                 (SELECT MAX(k.last_used_at) \
                    FROM models m \
                    JOIN api_keys k ON k.model_id = m.id \
                   WHERE m.inference_instance_id = i.id \
                     AND m.deployment_status IN ('active', 'deploying')), \
                 i.created_at \
               ) < NOW() - make_interval(secs => $1)",
    )
    .bind(idle_after_secs as f64)
    .fetch_all(db)
    .await?;

    let mut retired = 0;
    for (instance_id, name) in idle_instances {
        if retire_idle_instance(db, instance_id).await? {
            retired += 1;
            tracing::warn!(instance_id = %instance_id, name = %name, "Scaled idle serving instance to zero");
        }
    }
    Ok(retired)
}

/// Undeploy an instance's models and retire it, transactionally. Returns whether
/// the instance was retired (false if it already left the `ready` state).
async fn retire_idle_instance(db: &PgPool, instance_id: Uuid) -> Result<bool, sqlx::Error> {
    let mut tx = db.begin().await?;

    let retired = sqlx::query(
        "UPDATE inference_instances \
         SET lifecycle_state = 'retired', active_adapter_count = 0, updated_at = NOW() \
         WHERE id = $1 AND lifecycle_state = 'ready'",
    )
    .bind(instance_id)
    .execute(&mut *tx)
    .await?;

    if retired.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    sqlx::query(
        "UPDATE models \
         SET deployment_status = 'inactive', inference_instance_id = NULL, updated_at = NOW() \
         WHERE inference_instance_id = $1 AND deployment_status IN ('active', 'deploying')",
    )
    .bind(instance_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

/// Whether the orphaned-object sweep is enabled (a positive grace period).
fn orphan_sweep_enabled(older_than_secs: i64) -> bool {
    older_than_secs > 0
}

/// Delete the uploaded source object of documents that have been `failed`
/// longer than the grace period, then clear their `storage_path` so they are
/// not re-swept. Returns the number of objects reclaimed. Disabled when
/// `older_than_secs <= 0`.
///
/// Ordering is delete-then-clear: if the process dies after the object delete
/// but before the DB update, the next pass re-deletes (a missing object is
/// treated as success) and clears the path — no leak, no lost pointer.
pub async fn sweep_orphaned_document_objects(
    db: &PgPool,
    storage: &impl ObjectStorage,
    older_than_secs: i64,
) -> Result<usize, sqlx::Error> {
    if !orphan_sweep_enabled(older_than_secs) {
        return Ok(0);
    }

    let candidates: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, storage_path \
         FROM documents \
         WHERE status = 'failed' AND storage_path <> '' \
           AND updated_at < NOW() - make_interval(secs => $1)",
    )
    .bind(older_than_secs as f64)
    .fetch_all(db)
    .await?;

    let mut swept = 0;
    for (id, storage_path) in candidates {
        match storage.delete(&storage_path).await {
            Ok(()) | Err(StorageError::NotFound { .. }) => {}
            Err(e) => {
                tracing::warn!(document_id = %id, error = %e, "Failed to delete orphaned document object");
                continue;
            }
        }

        let cleared = sqlx::query(
            "UPDATE documents SET storage_path = '', updated_at = NOW() \
             WHERE id = $1 AND status = 'failed'",
        )
        .bind(id)
        .execute(db)
        .await?;

        if cleared.rows_affected() > 0 {
            swept += 1;
            tracing::info!(document_id = %id, "Reclaimed orphaned document object");
        }
    }
    Ok(swept)
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

/// Whether stuck-deploy reaping is enabled (a positive timeout).
fn deploy_reaping_enabled(stuck_after_secs: i64) -> bool {
    stuck_after_secs > 0
}

/// A candidate deployment pinned in `deploying`.
#[derive(sqlx::FromRow)]
struct StuckDeployment {
    id: Uuid,
    tenant_id: Uuid,
    inference_instance_id: Option<Uuid>,
}

/// Reap models pinned in `deploying` past the threshold. Returns the count.
///
/// A model enters `deploying` at the start of a deploy and flips to `active`
/// only once its adapter is loaded on the serving engine. Deploys are
/// synchronous (there is no background workflow to consult, unlike training —
/// so elapsed time is the sole abandonment signal); a model still `deploying`
/// well past the threshold means the deploy request died mid-flight. Such a
/// model is reset to the terminal `undeployed` state and any inference-instance
/// slot it claimed is released, reclaiming the pinned capacity and unblocking
/// redeploys. Disabled when `stuck_after_secs <= 0`.
pub async fn reap_stuck_deploying_models(
    db: &PgPool,
    stuck_after_secs: i64,
) -> Result<usize, sqlx::Error> {
    if !deploy_reaping_enabled(stuck_after_secs) {
        return Ok(0);
    }

    let candidates = sqlx::query_as::<_, StuckDeployment>(
        "SELECT id, tenant_id, inference_instance_id \
         FROM models \
         WHERE deployment_status = 'deploying' \
           AND updated_at < NOW() - make_interval(secs => $1)",
    )
    .bind(stuck_after_secs as f64)
    .fetch_all(db)
    .await?;

    let mut reaped = 0;
    for deployment in candidates {
        if reap_one_deployment(db, &deployment).await? {
            reaped += 1;
        }
    }
    Ok(reaped)
}

/// Reset one stuck deployment to `undeployed` and release its instance slot,
/// transactionally. Returns whether a row was reaped (false if it already left
/// the `deploying` state).
async fn reap_one_deployment(
    db: &PgPool,
    deployment: &StuckDeployment,
) -> Result<bool, sqlx::Error> {
    let mut tx = db.begin().await?;

    let updated = sqlx::query(
        "UPDATE models \
         SET deployment_status = 'undeployed', \
             inference_instance_id = NULL, \
             deployment_config = jsonb_build_object( \
                 'reaped', true, \
                 'error', 'Deploy did not complete; deployment reaped'), \
             updated_at = NOW() \
         WHERE id = $1 AND deployment_status = 'deploying'",
    )
    .bind(deployment.id)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }

    // Release the multi-instance slot the stuck deploy claimed. Single-instance
    // deploys carry no `inference_instance_id` — their global adapter cap counts
    // `deploying`/`active` models, so flipping to `undeployed` above already
    // frees that capacity.
    if let Some(instance_id) = deployment.inference_instance_id {
        sqlx::query(
            "UPDATE inference_instances \
             SET active_adapter_count = GREATEST(active_adapter_count - 1, 0), \
                 updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(instance_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    tracing::warn!(
        model_id = %deployment.id,
        tenant_id = %deployment.tenant_id,
        instance_id = ?deployment.inference_instance_id,
        "Reaped stuck deploying model"
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{
        REAPED_OUTCOME, STUCK_JOB_PREDICATE, StuckJob, deploy_reaping_enabled,
        idle_reaping_enabled, orphan_sweep_enabled, reap_update_sql, reaped_message,
        status_indicates_running, stuck_job_select_sql, teacher_serving_share,
    };

    fn job(status: &str, extraction: Option<&str>) -> StuckJob {
        StuckJob {
            id: uuid::Uuid::nil(),
            tenant_id: uuid::Uuid::nil(),
            status: status.into(),
            started_at: None,
            gpu_class: None,
            temporal_workflow_id: None,
            mode: "distill".into(),
            method: "lora".into(),
            base_model: "m".into(),
            teacher_extraction_status: extraction.map(str::to_string),
            hyperparams: serde_json::json!({}),
        }
    }

    /// The cap sums `extraction` and `teacher_serving`. A reaped improve pass
    /// billed wholly as `training` is teacher GPU time nobody counts — no revenue
    /// lost, but a tenant could cancel near completion, forever.
    #[test]
    fn a_reaped_improve_pass_is_split_so_the_teacher_budget_sees_it() {
        let mut improve = job("training", None);
        improve.gpu_class = Some("a10080gb_dual".to_string());
        improve.hyperparams = serde_json::json!({"distill_method": "on_policy"});

        let share = teacher_serving_share(improve.gpu_class.as_deref(), &improve.hyperparams);

        assert_eq!(share, 0.5);
    }

    #[test]
    fn a_reaped_ordinary_run_is_billed_whole() {
        let mut plain = job("training", None);
        plain.gpu_class = Some("a10080gb".to_string());

        assert_eq!(
            teacher_serving_share(plain.gpu_class.as_deref(), &plain.hyperparams),
            0.0
        );
    }

    /// The worker uses this word for a run that failed. Reaping is a failure the
    /// worker never got to report, so both writes address one ledger row.
    #[test]
    fn a_reaped_run_is_billed_under_the_outcome_a_worker_would_have_used() {
        assert_eq!(REAPED_OUTCOME, "failed");
    }

    #[test]
    fn the_select_and_the_update_agree_on_what_is_stuck() {
        assert!(stuck_job_select_sql().contains(STUCK_JOB_PREDICATE));
        assert!(reap_update_sql().contains(STUCK_JOB_PREDICATE));
    }

    #[test]
    fn a_job_abandoned_while_its_teacher_scored_is_reapable() {
        assert!(STUCK_JOB_PREDICATE.contains("status = 'pending'"));
        assert!(
            STUCK_JOB_PREDICATE.contains("teacher_extraction_status IN ('running', 'completed')")
        );
    }

    #[test]
    fn a_job_reaped_before_training_started_does_not_blame_the_trainer() {
        assert!(reaped_message(&job("pending", Some("running"))).contains("teacher scoring pass"));
        assert!(
            reaped_message(&job("pending", Some("running"))).contains("before training started")
        );
        assert!(reaped_message(&job("training", None)).contains("Training worker"));
        assert!(reaped_message(&job("training", Some("completed"))).contains("Training worker"));
    }

    #[test]
    fn a_job_whose_scoring_finished_is_not_blamed_on_the_scoring_pass() {
        let message = reaped_message(&job("pending", Some("completed")));
        assert!(message.contains("after the teacher finished scoring"));
        assert!(message.contains("before training started"));
        assert!(!message.contains("scoring pass stopped responding"));
    }

    #[test]
    fn idle_reaping_disabled_by_default() {
        assert!(!idle_reaping_enabled(0));
        assert!(!idle_reaping_enabled(-1));
        assert!(idle_reaping_enabled(1));
        assert!(idle_reaping_enabled(1800));
    }

    #[test]
    fn deploy_reaping_disabled_on_non_positive_timeout() {
        assert!(!deploy_reaping_enabled(0));
        assert!(!deploy_reaping_enabled(-1));
        assert!(deploy_reaping_enabled(1));
        assert!(deploy_reaping_enabled(600));
    }

    #[test]
    fn orphan_sweep_toggles_on_positive_grace() {
        assert!(!orphan_sweep_enabled(0));
        assert!(!orphan_sweep_enabled(-1));
        assert!(orphan_sweep_enabled(1));
        assert!(orphan_sweep_enabled(604_800));
    }

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
