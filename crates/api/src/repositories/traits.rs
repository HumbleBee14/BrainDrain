use chrono::{DateTime, Utc};
use platform_db::models::{
    ApiKey, AuditLog, BillingEvent, DataGuide, Dataset, Document, Evaluation, InferenceInstance,
    InferenceSample, Invitation, Model, ModelExport, NotificationDelivery, NotificationPreference,
    Project, TeamMember, Tenant, TrainingJob,
};
use platform_shared::enums::{
    DatasetStatus, DeploymentStatus, DocumentStatus, EvaluationStatus,
    InferenceInstanceHealthStatus, InferenceInstanceLifecycleState, TrainingJobStatus,
};
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::billing_event_repo::{InferenceUsageDay, UsageSummary};
use crate::services::teacher::billing::TeacherSpendReservation;

/// Convenience type alias for boxed futures (used by repository trait methods).
///
/// Repositories use `BoxFuture` instead of `impl Future` because they are stored
/// as `Arc<dyn XxxRepository>` in AppState — dynamic dispatch requires object safety,
/// which `impl Future` return types don't provide.
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Contract for project database operations.
///
/// All queries enforce multi-tenancy via `tenant_id`.
pub trait ProjectRepository: Send + Sync {
    #[allow(dead_code)]
    fn create(
        &self,
        tenant_id: Uuid,
        name: &str,
        description: Option<&str>,
        task_type: Option<&str>,
    ) -> BoxFuture<'_, AppResult<Project>>;

    /// Atomic create with plan limit enforcement.
    /// Inserts only if current count < max_count. Returns None if limit exceeded.
    fn create_with_limit(
        &self,
        tenant_id: Uuid,
        name: &str,
        description: Option<&str>,
        task_type: Option<&str>,
        max_count: i64,
    ) -> BoxFuture<'_, AppResult<Option<Project>>>;

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Project>>>;

    fn list(
        &self,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Project>>>;

    fn count(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    fn update(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        name: Option<&str>,
        description: Option<&str>,
        task_type: Option<&str>,
    ) -> BoxFuture<'_, AppResult<Option<Project>>>;

    fn update_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: &str,
    ) -> BoxFuture<'_, AppResult<Option<Project>>>;

    /// Hard-delete a project, cascading documents, datasets, runs and models.
    /// Callers must erase its stored objects first.
    fn delete(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<bool>>;
}

/// Contract for document database operations.
pub trait DocumentRepository: Send + Sync {
    fn create(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        filename: &str,
        file_size: i64,
        mime_type: &str,
        storage_path: &str,
    ) -> BoxFuture<'_, AppResult<Document>>;

    fn list_by_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Document>>>;

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        document_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Document>>>;

    fn list_by_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: DocumentStatus,
    ) -> BoxFuture<'_, AppResult<Vec<Document>>>;

    fn count_by_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: DocumentStatus,
    ) -> BoxFuture<'_, AppResult<i64>>;

    #[allow(dead_code)]
    fn update_status(
        &self,
        tenant_id: Uuid,
        document_id: Uuid,
        status: DocumentStatus,
        error_message: Option<&str>,
    ) -> BoxFuture<'_, AppResult<bool>>;

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    /// Count all documents across all projects for a tenant.
    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    /// Hard-delete a document row. Returns whether a row was deleted.
    fn delete(&self, tenant_id: Uuid, document_id: Uuid) -> BoxFuture<'_, AppResult<bool>>;
}

/// Contract for dataset database operations.
pub trait DatasetRepository: Send + Sync {
    fn list_by_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Dataset>>>;

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Dataset>>>;

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    fn count_by_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: DatasetStatus,
    ) -> BoxFuture<'_, AppResult<i64>>;

    /// Sum of generated training pairs across a tenant's datasets (pair-quota accounting).
    fn sum_pair_count(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    fn update_status(
        &self,
        tenant_id: Uuid,
        dataset_id: Uuid,
        status: DatasetStatus,
    ) -> BoxFuture<'_, AppResult<Option<Dataset>>>;

    /// Insert a dataset row for an imported (rather than generated) dataset.
    /// The row enters `review_pending` so it goes through the same approve/
    /// reject flow as a generated one.
    #[allow(clippy::too_many_arguments)]
    fn create_imported(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        dataset_id: Uuid,
        name: String,
        storage_path: String,
        pair_count: i32,
        stats: serde_json::Value,
        size_bytes: i64,
    ) -> BoxFuture<'_, AppResult<Dataset>>;

    /// Reserve a `generating` row before starting a refine run, so the run is
    /// visible while it works and has somewhere to record a failure.
    fn create_generating(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        dataset_id: Uuid,
        name: String,
        config: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<Dataset>>;

    fn mark_failed(
        &self,
        tenant_id: Uuid,
        dataset_id: Uuid,
        error: String,
    ) -> BoxFuture<'_, AppResult<Option<Dataset>>>;

    /// Fail `generating` rows whose run died without reporting: a terminated
    /// worker never gets to mark_failed, leaving the row generating forever.
    fn reap_stale_generating(&self, stale_minutes: i64) -> BoxFuture<'_, AppResult<i64>>;
}

/// Contract for data guide database operations.
///
/// Tracks the guided synthesis session (facets → preview → guidance → dataset)
/// that precedes dataset generation. One active session per project.
///
/// Not yet wired into `AppState` — the service layer consuming this trait
/// lands in a later task, so `#[allow(dead_code)]` is temporary.
#[allow(dead_code)]
pub trait DataGuideRepository: Send + Sync {
    fn create(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        task_type: &str,
    ) -> BoxFuture<'_, AppResult<DataGuide>>;

    fn get(&self, tenant_id: Uuid, id: Uuid) -> BoxFuture<'_, AppResult<Option<DataGuide>>>;

    /// Latest data guide session for a project (ordered by created_at DESC).
    fn get_for_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<DataGuide>>>;

    /// The guide that produced a given dataset, if any. Used to recover a
    /// model's system prompt at deploy time (model → training job → dataset →
    /// guide). Returns None for datasets not built via a data guide.
    fn get_by_dataset_id(
        &self,
        tenant_id: Uuid,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<DataGuide>>>;

    fn update_status(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        status: &str,
    ) -> BoxFuture<'_, AppResult<()>>;

    /// Return a finished or failed guide to `draft` so the session can be run
    /// again, keeping the guidance and system prompt the user wrote.
    fn reset_to_draft(
        &self,
        tenant_id: Uuid,
        id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<DataGuide>>>;

    fn update_facets(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        facets: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<()>>;

    /// Overwrites `preview_samples` with the rating-updated array computed by the service.
    fn apply_ratings(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        preview_samples: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<()>>;

    /// Updates guidance (and refinement history), and optionally the system
    /// prompt. `system_prompt: None` leaves the stored value unchanged;
    /// `Some("")` clears it.
    fn update_guidance(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        guidance: &str,
        system_prompt: Option<&str>,
        refinement_history: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<()>>;

    fn set_dataset_id(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, AppResult<()>>;
}

/// Contract for training job database operations.
///
/// `reservation` on the create methods is the teacher-GPU spend reservation an
/// on-policy run writes with its own row: the cap re-check, the job insert and
/// the reservation insert commit together under a per-tenant lock, so two
/// concurrent admissions cannot both read the budget before either joins it.
/// `Err(Forbidden)` when the re-check refuses — nothing is created then.
#[allow(clippy::too_many_arguments)]
pub trait TrainingJobRepository: Send + Sync {
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
    ) -> BoxFuture<'_, AppResult<TrainingJob>>;

    /// Atomic create with plan limit enforcement. Failed and cancelled runs
    /// release their slot. Returns None if the limit is hit.
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
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>>;

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>>;

    fn list_by_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<TrainingJob>>>;

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    fn count_by_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: TrainingJobStatus,
    ) -> BoxFuture<'_, AppResult<i64>>;

    fn update_workflow_id(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
        workflow_id: &str,
    ) -> BoxFuture<'_, AppResult<bool>>;

    fn cancel(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>>;

    /// Cancel a RUNNING job: transition `training` → `cancelled`, record the
    /// actual GPU cost, and enqueue the billing row(s) in the same transaction.
    /// Returns None if the job is not currently running (so the caller does not
    /// double-bill on repeated cancels). The workflow must already be terminated.
    ///
    /// `teacher_share` splits the bill when a teacher shared the container — see
    /// `teacher::serving_cost`. Zero for every run that had none.
    fn finalize_cancelled(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
        actual_cost: f64,
        gpu_seconds: i32,
        metadata: serde_json::Value,
        teacher_share: f64,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>>;

    /// Set a job's status to cost_approval (only from pending).
    fn set_cost_approval(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>>;

    /// Approve a cost_approval job — transitions to pending so it can be started.
    fn approve_cost(
        &self,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<TrainingJob>>>;

    /// Count all training jobs for a tenant.
    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    /// Count training jobs by status for a tenant (across all projects).
    fn count_by_tenant_status(
        &self,
        tenant_id: Uuid,
        status: TrainingJobStatus,
    ) -> BoxFuture<'_, AppResult<i64>>;
}

/// Contract for model database operations.
pub trait ModelRepository: Send + Sync {
    fn get_by_id(&self, tenant_id: Uuid, model_id: Uuid)
    -> BoxFuture<'_, AppResult<Option<Model>>>;

    fn list_by_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Model>>>;

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    fn count_by_deployment_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: DeploymentStatus,
    ) -> BoxFuture<'_, AppResult<i64>>;

    /// Count all actively deployed adapters sharing the same base model (across all tenants).
    /// Used to enforce the `--max-loras` limit before loading a new adapter.
    fn count_active_by_base_model(&self, base_model: &str) -> BoxFuture<'_, AppResult<i64>>;

    /// Atomically claim a deployment slot: set status to 'deploying' only if the
    /// count of active adapters for this base_model is below max_loras.
    /// Returns true if the slot was claimed, false if the limit is reached.
    /// Concurrency-safe: uses a single UPDATE ... WHERE subquery (no TOCTOU race).
    fn claim_deployment_slot(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        base_model: &str,
        max_loras: i64,
    ) -> BoxFuture<'_, AppResult<bool>>;

    /// Reset models stuck in 'deploying' for longer than the given minutes back to 'undeployed'.
    /// Prevents capacity leaks when the API dies mid-deploy before cleanup runs.
    fn reap_stale_deployments(&self, stale_minutes: i64) -> BoxFuture<'_, AppResult<i64>>;

    #[allow(dead_code)]
    fn update_deployment_status(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        status: DeploymentStatus,
    ) -> BoxFuture<'_, AppResult<Option<Model>>>;

    #[allow(dead_code)]
    fn update_eval_scores(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        scores: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<bool>>;

    /// Count all models for a tenant.
    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    /// Hard-delete the run that produced a model, cascading the model row and
    /// everything hanging off it. A run and its model are one unit.
    fn delete_with_training_job(
        &self,
        tenant_id: Uuid,
        training_job_id: Uuid,
    ) -> BoxFuture<'_, AppResult<bool>>;

    /// Count models by deployment status for a tenant (across all projects).
    fn count_by_tenant_deployment_status(
        &self,
        tenant_id: Uuid,
        status: DeploymentStatus,
    ) -> BoxFuture<'_, AppResult<i64>>;

    /// List all versions of models sharing the same base_model within a project.
    fn list_versions(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        base_model: &str,
    ) -> BoxFuture<'_, AppResult<Vec<Model>>>;

    /// Get the highest version number for a given base_model within a project.
    /// Used by the Python training worker to auto-increment on model creation.
    #[allow(dead_code)]
    fn get_max_version(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        base_model: &str,
    ) -> BoxFuture<'_, AppResult<i32>>;

    /// Toggle production traffic capture (data flywheel) for a model.
    fn set_capture_traffic(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        enabled: bool,
    ) -> BoxFuture<'_, AppResult<bool>>;
}

/// Contract for captured inference traffic (data flywheel).
pub trait InferenceSampleRepository: Send + Sync {
    /// Insert a captured request/response pair. The caller supplies the id so
    /// it can be returned to the API client (as `x-sample-id`) before the
    /// write completes.
    #[allow(clippy::too_many_arguments)]
    fn insert(
        &self,
        tenant_id: Uuid,
        sample_id: Uuid,
        model_id: Uuid,
        api_key_id: Option<Uuid>,
        messages: serde_json::Value,
        response: &str,
    ) -> BoxFuture<'_, AppResult<()>>;

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        sample_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<InferenceSample>>>;

    /// List samples for a model, newest first. `rating` filters to one rating
    /// value; `unrated_only` filters to samples with no rating yet.
    fn list_by_model(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        rating: Option<String>,
        unrated_only: bool,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<InferenceSample>>>;

    fn count_by_model(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        rating: Option<String>,
        unrated_only: bool,
    ) -> BoxFuture<'_, AppResult<i64>>;

    fn set_rating(
        &self,
        tenant_id: Uuid,
        sample_id: Uuid,
        rating: &str,
        comment: Option<String>,
    ) -> BoxFuture<'_, AppResult<bool>>;

    /// Stamp `promoted_at` on the given samples. Returns how many rows were
    /// updated (already-promoted rows are not re-stamped).
    fn mark_promoted(&self, tenant_id: Uuid, sample_ids: &[Uuid]) -> BoxFuture<'_, AppResult<u64>>;
}

/// Contract for global inference instance control-plane operations.
#[allow(clippy::too_many_arguments)]
pub trait InferenceInstanceRepository: Send + Sync {
    fn create(
        &self,
        name: &str,
        base_url: &str,
        backend_type: &str,
        gpu_class: Option<&str>,
        base_model: &str,
        max_adapters: i32,
        health_status: InferenceInstanceHealthStatus,
        lifecycle_state: InferenceInstanceLifecycleState,
        metadata: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<InferenceInstance>>;

    fn get_by_id(&self, id: Uuid) -> BoxFuture<'_, AppResult<Option<InferenceInstance>>>;

    fn list(&self) -> BoxFuture<'_, AppResult<Vec<InferenceInstance>>>;

    /// Atomically reserve one adapter slot on the least-loaded compatible instance.
    fn claim_slot(
        &self,
        backend_type: &str,
        base_model: &str,
    ) -> BoxFuture<'_, AppResult<Option<InferenceInstance>>>;

    fn release_slot(&self, id: Uuid) -> BoxFuture<'_, AppResult<()>>;

    fn update_health(
        &self,
        id: Uuid,
        health_status: InferenceInstanceHealthStatus,
    ) -> BoxFuture<'_, AppResult<Option<InferenceInstance>>>;

    fn update_lifecycle_state(
        &self,
        id: Uuid,
        lifecycle_state: InferenceInstanceLifecycleState,
    ) -> BoxFuture<'_, AppResult<Option<InferenceInstance>>>;

    /// Atomically retire an instance only if it has zero active adapters.
    /// Returns None if the instance doesn't exist or has active adapters.
    fn retire_if_empty(&self, id: Uuid) -> BoxFuture<'_, AppResult<Option<InferenceInstance>>>;

    /// Atomically delete an instance only if it has zero active adapters.
    /// Returns false if the instance doesn't exist or has active adapters.
    fn delete_if_empty(&self, id: Uuid) -> BoxFuture<'_, AppResult<bool>>;

    fn list_for_healthcheck(&self) -> BoxFuture<'_, AppResult<Vec<InferenceInstance>>>;

    fn reconcile_adapter_counts(&self) -> BoxFuture<'_, AppResult<u64>>;
}

/// Contract for evaluation database operations.
pub trait EvaluationRepository: Send + Sync {
    fn create(&self, tenant_id: Uuid, model_id: Uuid) -> BoxFuture<'_, AppResult<Evaluation>>;

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        eval_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Evaluation>>>;

    fn list_by_model(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Evaluation>>>;

    fn count_by_model(&self, tenant_id: Uuid, model_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    /// Scores JSON of the model's most recent *completed* evaluation, or `None`
    /// when the model has no completed evaluation. Used by the deployment eval
    /// gate to read a model's quality evidence at deploy time.
    fn latest_completed_scores(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<serde_json::Value>>>;

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    fn count_by_project_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: EvaluationStatus,
    ) -> BoxFuture<'_, AppResult<i64>>;

    fn update_workflow_id(
        &self,
        tenant_id: Uuid,
        eval_id: Uuid,
        workflow_id: &str,
    ) -> BoxFuture<'_, AppResult<bool>>;

    /// Count all evaluations for a tenant.
    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;
}

/// Contract for API key database operations.
///
/// `get_by_hash` and `update_last_used` don't require `tenant_id` —
/// auth by hash must work without tenant context.
#[allow(clippy::too_many_arguments)]
pub trait ApiKeyRepository: Send + Sync {
    fn create(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        name: &str,
        key_prefix: &str,
        key_hash: &str,
        rate_limit: i32,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> BoxFuture<'_, AppResult<ApiKey>>;

    fn get_by_hash(&self, key_hash: &str) -> BoxFuture<'_, AppResult<Option<ApiKey>>>;

    fn list_by_model(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Vec<ApiKey>>>;

    fn revoke(&self, tenant_id: Uuid, key_id: Uuid) -> BoxFuture<'_, AppResult<Option<ApiKey>>>;

    fn update_last_used(&self, key_id: Uuid) -> BoxFuture<'_, AppResult<()>>;
}

/// Contract for billing event database operations.
#[allow(clippy::too_many_arguments, dead_code)]
pub trait BillingEventRepository: Send + Sync {
    fn create(
        &self,
        tenant_id: Uuid,
        operation: &str,
        resource_id: Option<Uuid>,
        tokens_in: i64,
        tokens_out: i64,
        gpu_seconds: i32,
        cost_usd: f64,
        metadata: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<BillingEvent>>;

    fn list_by_tenant(
        &self,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<BillingEvent>>>;

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    #[allow(dead_code)]
    fn sum_by_resource(
        &self,
        tenant_id: Uuid,
        resource_id: Uuid,
    ) -> BoxFuture<'_, AppResult<UsageSummary>>;

    /// Aggregate daily costs over the last N days.
    fn usage_by_day(
        &self,
        tenant_id: Uuid,
        days: i32,
    ) -> BoxFuture<'_, AppResult<Vec<(String, f64)>>>;

    /// Aggregate lifetime cost and event count per operation, most expensive first.
    fn usage_by_operation(
        &self,
        tenant_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Vec<(String, f64, i64)>>>;

    /// Aggregate total cost, tokens_in, and tokens_out for a tenant.
    fn usage_totals(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<(f64, i64, i64)>>;

    /// Total committed cost for a tenant since a timestamp (spend-cap accounting).
    fn sum_cost_since(
        &self,
        tenant_id: Uuid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> BoxFuture<'_, AppResult<f64>>;

    /// Cost committed to the ledger plus cost still in `billing_outbox`
    /// awaiting delivery — reservations for runs on a GPU and terminal charges
    /// the relay has not moved yet — since a timestamp, restricted to the given
    /// operations. Teacher-GPU spend-cap accounting is its own budget line,
    /// separate from total tenant spend.
    ///
    /// One read on purpose: the relay moves a row from outbox to ledger in a
    /// single commit, so two separate reads can miss it in both — a delivered
    /// row absent from the earlier ledger read and no longer undelivered by the
    /// later outbox read — under-counting spend at the exact moment it becomes
    /// real.
    fn sum_delivered_and_in_flight_cost_since(
        &self,
        tenant_id: Uuid,
        operations: &[String],
        since: chrono::DateTime<chrono::Utc>,
    ) -> BoxFuture<'_, AppResult<f64>>;

    /// Inference usage breakdown by day (last N days).
    fn inference_usage_by_day(
        &self,
        tenant_id: Uuid,
        days: i32,
    ) -> BoxFuture<'_, AppResult<Vec<InferenceUsageDay>>>;
}

/// Contract for audit log database operations.
///
/// Append-only, tenant-scoped. Follows the same pattern as BillingEventRepository.
#[allow(clippy::too_many_arguments)]
pub trait AuditLogRepository: Send + Sync {
    fn create(
        &self,
        tenant_id: Uuid,
        actor_id: &str,
        action: &str,
        resource_type: &str,
        resource_id: Option<Uuid>,
        metadata: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<AuditLog>>;

    fn list_by_tenant(
        &self,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<AuditLog>>>;

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    fn list_by_resource(
        &self,
        tenant_id: Uuid,
        resource_type: &str,
        resource_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<AuditLog>>>;

    fn count_by_resource(
        &self,
        tenant_id: Uuid,
        resource_type: &str,
        resource_id: Uuid,
    ) -> BoxFuture<'_, AppResult<i64>>;

    fn list_filtered(
        &self,
        tenant_id: Uuid,
        action: Option<&str>,
        resource_type: Option<&str>,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<AuditLog>>>;

    fn count_filtered(
        &self,
        tenant_id: Uuid,
        action: Option<&str>,
        resource_type: Option<&str>,
    ) -> BoxFuture<'_, AppResult<i64>>;
}

/// Contract for team member database operations.
#[allow(dead_code)]
pub trait TeamMemberRepository: Send + Sync {
    fn create(
        &self,
        tenant_id: Uuid,
        user_id: &str,
        email: &str,
        role: &str,
        invited_by: Option<&str>,
    ) -> BoxFuture<'_, AppResult<TeamMember>>;

    fn get_by_user(
        &self,
        tenant_id: Uuid,
        user_id: &str,
    ) -> BoxFuture<'_, AppResult<Option<TeamMember>>>;

    fn get_role(&self, tenant_id: Uuid, user_id: &str) -> BoxFuture<'_, AppResult<Option<String>>>;

    fn list_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<Vec<TeamMember>>>;

    /// O(1) membership check — avoids fetching all members just to scan for an email.
    fn email_exists(&self, tenant_id: Uuid, email: &str) -> BoxFuture<'_, AppResult<bool>>;

    fn update_role(
        &self,
        tenant_id: Uuid,
        user_id: &str,
        role: &str,
    ) -> BoxFuture<'_, AppResult<TeamMember>>;

    fn remove(&self, tenant_id: Uuid, user_id: &str) -> BoxFuture<'_, AppResult<()>>;

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;
}

/// Contract for invitation database operations.
#[allow(clippy::too_many_arguments)]
pub trait InvitationRepository: Send + Sync {
    fn create(
        &self,
        tenant_id: Uuid,
        email: &str,
        role: &str,
        token: &str,
        invited_by: &str,
        expires_at: DateTime<Utc>,
    ) -> BoxFuture<'_, AppResult<Invitation>>;

    /// Atomic create with plan limit enforcement.
    /// Inserts only if current team member count for tenant < max_members.
    /// Returns None if limit exceeded.
    fn create_with_limit(
        &self,
        tenant_id: Uuid,
        email: &str,
        role: &str,
        token: &str,
        invited_by: &str,
        expires_at: DateTime<Utc>,
        max_members: i64,
    ) -> BoxFuture<'_, AppResult<Option<Invitation>>>;

    fn get_by_token(&self, token: &str) -> BoxFuture<'_, AppResult<Option<Invitation>>>;

    fn list_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<Vec<Invitation>>>;

    fn accept(&self, id: Uuid) -> BoxFuture<'_, AppResult<Invitation>>;

    fn revoke(&self, tenant_id: Uuid, id: Uuid) -> BoxFuture<'_, AppResult<Invitation>>;
}

/// Contract for tenant database operations.
///
/// Note: `get_by_id` and `get_by_stripe_customer` do not require `tenant_id`
/// as a filter because tenants are the root entity (the tenant IS the row).
#[allow(dead_code)]
pub trait TenantRepository: Send + Sync {
    fn get_by_id(&self, id: Uuid) -> BoxFuture<'_, AppResult<Option<Tenant>>>;

    fn update_stripe_customer(&self, id: Uuid, customer_id: &str) -> BoxFuture<'_, AppResult<()>>;

    fn update_subscription(
        &self,
        id: Uuid,
        subscription_id: &str,
        plan: &str,
        limits: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<()>>;

    fn get_by_stripe_customer(&self, customer_id: &str)
    -> BoxFuture<'_, AppResult<Option<Tenant>>>;

    fn get_plan_limits(&self, id: Uuid) -> BoxFuture<'_, AppResult<serde_json::Value>>;

    /// Get the settings JSONB for a tenant.
    fn get_settings(&self, id: Uuid) -> BoxFuture<'_, AppResult<serde_json::Value>>;

    /// Update the settings JSONB for a tenant (shallow merge at top level).
    fn update_settings(
        &self,
        id: Uuid,
        settings: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<()>>;

    /// Documents + datasets + adapters + exports. Rows with no recorded size
    /// contribute zero, so this is a floor.
    fn sum_storage_bytes(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    /// Delete a tenant row. Cascades all operational tables (tenant erasure).
    /// Returns whether a row was deleted (false if the tenant was already gone).
    /// Cross-tenant platform-admin operation — not tenant-scoped.
    fn delete(&self, id: Uuid) -> BoxFuture<'_, AppResult<bool>>;
}

/// Contract for notification database operations.
#[allow(dead_code)]
pub trait NotificationRepository: Send + Sync {
    fn list_preferences(
        &self,
        tenant_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Vec<NotificationPreference>>>;

    fn upsert_preference(
        &self,
        tenant_id: Uuid,
        channel: &str,
        event_type: &str,
        enabled: bool,
        config: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<NotificationPreference>>;

    fn get_enabled_preferences(
        &self,
        tenant_id: Uuid,
        event_type: &str,
    ) -> BoxFuture<'_, AppResult<Vec<NotificationPreference>>>;

    fn create_delivery(
        &self,
        tenant_id: Uuid,
        preference_id: Uuid,
        event_type: &str,
        channel: &str,
        payload: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<NotificationDelivery>>;

    fn list_deliveries(
        &self,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<NotificationDelivery>>>;

    fn count_deliveries(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    fn update_delivery_status(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> BoxFuture<'_, AppResult<()>>;

    fn get_delivery(
        &self,
        tenant_id: Uuid,
        delivery_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<NotificationDelivery>>>;

    fn get_preference(
        &self,
        tenant_id: Uuid,
        preference_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<NotificationPreference>>>;

    /// Atomically claim deliveries eligible for processing by the background
    /// worker: pending or under-limit failed rows whose backoff has elapsed.
    /// Uses `FOR UPDATE SKIP LOCKED` and leases each claimed row (pushes
    /// `next_retry_at` out by `lease_secs`) so concurrent workers don't
    /// double-send and a crashed worker's rows become eligible again after the
    /// lease expires. Oldest-first.
    fn claim_pending_deliveries(
        &self,
        max_attempts: i32,
        limit: i64,
        lease_secs: i64,
    ) -> BoxFuture<'_, AppResult<Vec<NotificationDelivery>>>;

    /// Recent in-app deliveries for the bell menu, newest first.
    fn list_in_app(
        &self,
        tenant_id: Uuid,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<NotificationDelivery>>>;

    /// Count unread in-app deliveries for the bell badge.
    fn count_unread_in_app(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>>;

    /// Mark a single in-app delivery read. Returns whether a row was updated.
    fn mark_in_app_read(&self, tenant_id: Uuid, id: Uuid) -> BoxFuture<'_, AppResult<bool>>;

    /// Mark every unread in-app delivery read. Returns the number updated.
    fn mark_all_in_app_read(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<u64>>;
}

/// Contract for model export database operations.
#[allow(dead_code)]
pub trait ExportRepository: Send + Sync {
    fn create(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        format: &str,
        quant_type: &str,
    ) -> BoxFuture<'_, AppResult<ModelExport>>;

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        export_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<ModelExport>>>;

    fn list_by_model(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Vec<ModelExport>>>;

    #[allow(clippy::too_many_arguments)]
    fn update_status(
        &self,
        tenant_id: Uuid,
        export_id: Uuid,
        status: &str,
        storage_path: Option<&str>,
        file_size_bytes: Option<i64>,
        error: Option<&str>,
    ) -> BoxFuture<'_, AppResult<bool>>;
}
