use platform_shared::enums::{DeploymentStatus, TrainingJobStatus};
use uuid::Uuid;

use crate::dto::dashboard::{ActivityEntry, DailyCost, DashboardStats, UsageSummary};
use crate::error::AppResult;
use crate::repositories::traits::{
    AuditLogRepository, BillingEventRepository, DocumentRepository, EvaluationRepository,
    ModelRepository, ProjectRepository, TrainingJobRepository,
};

/// Aggregates cross-entity stats for the tenant dashboard.
pub struct DashboardService;

impl DashboardService {
    /// Gather high-level counts across all projects for a tenant.
    ///
    /// **Pool exhaustion risk:** This method fires 7 parallel `COUNT(*)` queries,
    /// each holding a connection from the pool for the duration. Under high
    /// concurrency this can starve other requests of connections.
    ///
    /// Planned optimisation (not yet implemented):
    /// - **Short-term:** cache each count in Redis with a 30-60s TTL keyed per
    ///   tenant, so only the first request per window hits the database.
    /// - **Long-term:** collapse into a single query using
    ///   `COUNT(*) FILTER (WHERE ...)` expressions to reduce the round-trips to 1.
    pub async fn get_stats(
        project_repo: &dyn ProjectRepository,
        document_repo: &dyn DocumentRepository,
        training_job_repo: &dyn TrainingJobRepository,
        model_repo: &dyn ModelRepository,
        evaluation_repo: &dyn EvaluationRepository,
        tenant_id: Uuid,
    ) -> AppResult<DashboardStats> {
        let (
            total_projects,
            total_documents,
            total_training_jobs,
            active_training_jobs,
            total_models,
            deployed_models,
            total_evaluations,
        ) = tokio::try_join!(
            project_repo.count(tenant_id),
            document_repo.count_by_tenant(tenant_id),
            training_job_repo.count_by_tenant(tenant_id),
            training_job_repo.count_by_tenant_status(tenant_id, TrainingJobStatus::Training),
            model_repo.count_by_tenant(tenant_id),
            model_repo.count_by_tenant_deployment_status(tenant_id, DeploymentStatus::Active),
            evaluation_repo.count_by_tenant(tenant_id),
        )?;

        Ok(DashboardStats {
            total_projects,
            total_documents,
            total_training_jobs,
            active_training_jobs,
            total_models,
            deployed_models,
            total_evaluations,
        })
    }

    /// Aggregate billing usage with daily cost breakdown (last 30 days).
    pub async fn get_usage(
        billing_repo: &dyn BillingEventRepository,
        tenant_id: Uuid,
    ) -> AppResult<UsageSummary> {
        let (totals, daily, total_events) = tokio::try_join!(
            billing_repo.usage_totals(tenant_id),
            billing_repo.usage_by_day(tenant_id, 30),
            billing_repo.count_by_tenant(tenant_id),
        )?;

        let (total_cost_usd, total_tokens_in, total_tokens_out) = totals;

        let cost_by_day = daily
            .into_iter()
            .map(|(date, cost_usd)| DailyCost { date, cost_usd })
            .collect();

        Ok(UsageSummary {
            total_cost_usd,
            total_tokens_in,
            total_tokens_out,
            total_events,
            cost_by_day,
        })
    }

    /// Get recent activity from the audit log (last 10 entries).
    pub async fn get_activity(
        audit_repo: &dyn AuditLogRepository,
        tenant_id: Uuid,
    ) -> AppResult<Vec<ActivityEntry>> {
        let logs = audit_repo.list_by_tenant(tenant_id, 0, 10).await?;

        Ok(logs
            .into_iter()
            .map(|log| ActivityEntry {
                id: log.id.to_string(),
                actor_id: log.actor_id,
                action: log.action,
                resource_type: log.resource_type,
                resource_id: log.resource_id.map(|id| id.to_string()),
                created_at: log.created_at.to_rfc3339(),
            })
            .collect())
    }
}
