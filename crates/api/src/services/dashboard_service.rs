use platform_shared::enums::{DeploymentStatus, TrainingJobStatus};
use redis::AsyncCommands;
use uuid::Uuid;

use crate::dto::dashboard::{ActivityEntry, DailyCost, DashboardStats, UsageSummary};
use crate::error::AppResult;
use crate::repositories::traits::{
    AuditLogRepository, BillingEventRepository, DocumentRepository, EvaluationRepository,
    ModelRepository, ProjectRepository, TrainingJobRepository,
};

/// Redis TTL for dashboard stats cache.
const STATS_CACHE_TTL_SECS: u64 = 30;
/// Redis TTL for usage summary cache.
const USAGE_CACHE_TTL_SECS: u64 = 60;

/// Aggregates cross-entity stats for the tenant dashboard.
/// Results are cached in Redis per tenant to avoid 7 parallel COUNT queries per load.
pub struct DashboardService;

impl DashboardService {
    /// Gather high-level counts across all projects for a tenant.
    /// Cached in Redis for 30s per tenant.
    pub async fn get_stats(
        project_repo: &dyn ProjectRepository,
        document_repo: &dyn DocumentRepository,
        training_job_repo: &dyn TrainingJobRepository,
        model_repo: &dyn ModelRepository,
        evaluation_repo: &dyn EvaluationRepository,
        mut redis: redis::aio::ConnectionManager,
        tenant_id: Uuid,
    ) -> AppResult<DashboardStats> {
        let cache_key = format!("dashboard_stats:{tenant_id}");

        // Try cache first
        if let Ok(Some(json_str)) = redis.get::<_, Option<String>>(&cache_key).await
            && let Ok(stats) = serde_json::from_str::<DashboardStats>(&json_str)
        {
            return Ok(stats);
        }

        // Cache miss — run queries
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

        let stats = DashboardStats {
            total_projects,
            total_documents,
            total_training_jobs,
            active_training_jobs,
            total_models,
            deployed_models,
            total_evaluations,
        };

        // Write to cache (best-effort — don't fail the request)
        if let Ok(json_str) = serde_json::to_string(&stats) {
            let _: Result<(), _> = redis
                .set_ex(&cache_key, json_str, STATS_CACHE_TTL_SECS)
                .await;
        }

        Ok(stats)
    }

    /// Aggregate billing usage with daily cost breakdown (last 30 days).
    /// Cached in Redis for 60s per tenant.
    pub async fn get_usage(
        billing_repo: &dyn BillingEventRepository,
        mut redis: redis::aio::ConnectionManager,
        tenant_id: Uuid,
    ) -> AppResult<UsageSummary> {
        let cache_key = format!("dashboard_usage:{tenant_id}");

        // Try cache first
        if let Ok(Some(json_str)) = redis.get::<_, Option<String>>(&cache_key).await
            && let Ok(usage) = serde_json::from_str::<UsageSummary>(&json_str)
        {
            return Ok(usage);
        }

        // Cache miss — run queries
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

        let usage = UsageSummary {
            total_cost_usd,
            total_tokens_in,
            total_tokens_out,
            total_events,
            cost_by_day,
        };

        // Write to cache
        if let Ok(json_str) = serde_json::to_string(&usage) {
            let _: Result<(), _> = redis
                .set_ex(&cache_key, json_str, USAGE_CACHE_TTL_SECS)
                .await;
        }

        Ok(usage)
    }

    /// Get recent activity from the audit log (last 10 entries).
    /// Not cached — activity should always be fresh.
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
