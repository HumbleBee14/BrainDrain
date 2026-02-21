use platform_db::models::BillingEvent;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Repository for billing event database operations.
///
/// Billing events are append-only. All queries require `tenant_id`.
pub struct BillingEventRepo;

impl BillingEventRepo {
    /// Insert a new billing event.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        db: &PgPool,
        tenant_id: Uuid,
        operation: &str,
        resource_id: Option<Uuid>,
        tokens_in: i64,
        tokens_out: i64,
        gpu_seconds: i32,
        cost_usd: f64,
        metadata: serde_json::Value,
    ) -> Result<BillingEvent, AppError> {
        let event = sqlx::query_as::<_, BillingEvent>(
            r#"
            INSERT INTO billing_events
                (tenant_id, operation, resource_id, tokens_in, tokens_out, gpu_seconds, cost_usd, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(operation)
        .bind(resource_id)
        .bind(tokens_in)
        .bind(tokens_out)
        .bind(gpu_seconds)
        .bind(cost_usd)
        .bind(metadata)
        .fetch_one(db)
        .await?;

        Ok(event)
    }

    /// List billing events for a tenant, paginated.
    pub async fn list_by_tenant(
        db: &PgPool,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<BillingEvent>, AppError> {
        let events = sqlx::query_as::<_, BillingEvent>(
            r#"
            SELECT * FROM billing_events
            WHERE tenant_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(db)
        .await?;

        Ok(events)
    }

    /// Count billing events for a tenant.
    pub async fn count_by_tenant(db: &PgPool, tenant_id: Uuid) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM billing_events WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Aggregate usage by resource_id for a tenant.
    #[allow(dead_code)]
    pub async fn sum_by_resource(
        db: &PgPool,
        tenant_id: Uuid,
        resource_id: Uuid,
    ) -> Result<UsageSummary, AppError> {
        let row = sqlx::query_as::<_, UsageSummary>(
            r#"
            SELECT
                COALESCE(SUM(tokens_in), 0) AS total_tokens_in,
                COALESCE(SUM(tokens_out), 0) AS total_tokens_out,
                COALESCE(SUM(gpu_seconds), 0) AS total_gpu_seconds,
                COALESCE(SUM(cost_usd), 0)::FLOAT8 AS total_cost_usd,
                COUNT(*) AS event_count
            FROM billing_events
            WHERE tenant_id = $1 AND resource_id = $2
            "#,
        )
        .bind(tenant_id)
        .bind(resource_id)
        .fetch_one(db)
        .await?;

        Ok(row)
    }
}

/// Aggregated usage summary for a resource.
#[allow(dead_code)]
#[derive(Debug, sqlx::FromRow, serde::Serialize)]
pub struct UsageSummary {
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
    pub total_gpu_seconds: i64,
    pub total_cost_usd: f64,
    pub event_count: i64,
}
