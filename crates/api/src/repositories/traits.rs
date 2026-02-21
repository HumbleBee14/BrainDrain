use chrono::{DateTime, Utc};
use platform_db::models::{
    ApiKey, AuditLog, BillingEvent, Dataset, Document, Evaluation, Invitation, Model,
    NotificationDelivery, NotificationPreference, Project, TeamMember, Tenant, TrainingJob,
};
use platform_shared::enums::{
    DatasetStatus, DeploymentStatus, DocumentStatus, EvaluationStatus, TrainingJobStatus,
};
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::billing_event_repo::UsageSummary;

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
    fn create(
        &self,
        tenant_id: Uuid,
        name: &str,
        description: Option<&str>,
        task_type: Option<&str>,
    ) -> BoxFuture<'_, AppResult<Project>>;

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
}

/// Contract for training job database operations.
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
    ) -> BoxFuture<'_, AppResult<TrainingJob>>;

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

    fn update_deployment_status(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        status: DeploymentStatus,
    ) -> BoxFuture<'_, AppResult<Option<Model>>>;

    fn update_deployment(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        status: DeploymentStatus,
        config: serde_json::Value,
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

    /// Count models by deployment status for a tenant (across all projects).
    fn count_by_tenant_deployment_status(
        &self,
        tenant_id: Uuid,
        status: DeploymentStatus,
    ) -> BoxFuture<'_, AppResult<i64>>;
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
#[allow(clippy::too_many_arguments)]
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

    /// Aggregate total cost, tokens_in, and tokens_out for a tenant.
    fn usage_totals(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<(f64, i64, i64)>>;
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
        id: Uuid,
        status: &str,
        error: Option<&str>,
    ) -> BoxFuture<'_, AppResult<()>>;
}
