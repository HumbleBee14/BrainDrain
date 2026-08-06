use platform_db::models::BillingEvent;
use platform_db::tenant::begin_tenant_tx;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BillingEventRepository, BoxFuture};

/// Spend committed to the ledger plus spend still undelivered in the outbox,
/// for one tenant across a set of operations, since a timestamp.
///
/// One statement — one snapshot. The relay moves a row from outbox to ledger in
/// a single commit, so reading the two tables separately can count that row
/// zero times: absent from an earlier ledger read, already delivered by a later
/// outbox read. Shared with the admission re-check in `training_job_repo`,
/// which runs the same sum inside its own locked transaction, so the two
/// enforcement points cannot drift apart.
///
/// Binds: `$1` tenant_id, `$2` operations, `$3` since.
pub(crate) const DELIVERED_AND_IN_FLIGHT_COST_SQL: &str = "SELECT COALESCE((SELECT SUM(cost_usd) FROM billing_events \
        WHERE tenant_id = $1 AND operation = ANY($2) AND created_at >= $3), 0)::FLOAT8 \
          + COALESCE((SELECT SUM(cost_usd) FROM billing_outbox \
        WHERE tenant_id = $1 AND operation = ANY($2) AND created_at >= $3 \
          AND delivered_at IS NULL), 0)::FLOAT8";

/// Daily inference usage breakdown.
#[derive(Debug, sqlx::FromRow, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct InferenceUsageDay {
    pub date: String,
    pub request_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost_usd: f64,
}

/// Aggregated usage summary for a resource.
#[derive(Debug, sqlx::FromRow, serde::Serialize)]
#[allow(dead_code)]
pub struct UsageSummary {
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
    pub total_gpu_seconds: i64,
    pub total_cost_usd: f64,
    pub event_count: i64,
}

/// PostgreSQL implementation of the billing event repository.
///
/// Billing events are append-only. All queries require `tenant_id`.
pub struct PgBillingEventRepo {
    db: PgPool,
}

impl PgBillingEventRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl BillingEventRepository for PgBillingEventRepo {
    #[allow(clippy::too_many_arguments)]
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
    ) -> BoxFuture<'_, AppResult<BillingEvent>> {
        let operation = operation.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let event = sqlx::query_as::<_, BillingEvent>(
                r#"
                INSERT INTO billing_events
                    (tenant_id, operation, resource_id, tokens_in, tokens_out, gpu_seconds, cost_usd, metadata)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(&operation)
            .bind(resource_id)
            .bind(tokens_in)
            .bind(tokens_out)
            .bind(gpu_seconds)
            .bind(cost_usd)
            .bind(metadata)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(event)
        })
    }

    fn list_by_tenant(
        &self,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<BillingEvent>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
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
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(events)
        })
    }

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM billing_events WHERE tenant_id = $1",
            )
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }

    fn sum_by_resource(
        &self,
        tenant_id: Uuid,
        resource_id: Uuid,
    ) -> BoxFuture<'_, AppResult<UsageSummary>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
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
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(row)
        })
    }

    fn sum_cost_since(
        &self,
        tenant_id: Uuid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> BoxFuture<'_, AppResult<f64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let total = sqlx::query_scalar::<_, f64>(
                "SELECT COALESCE(SUM(cost_usd), 0)::FLOAT8 FROM billing_events \
                 WHERE tenant_id = $1 AND created_at >= $2",
            )
            .bind(tenant_id)
            .bind(since)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(total)
        })
    }

    fn sum_delivered_and_in_flight_cost_since(
        &self,
        tenant_id: Uuid,
        operations: &[String],
        since: chrono::DateTime<chrono::Utc>,
    ) -> BoxFuture<'_, AppResult<f64>> {
        let operations = operations.to_vec();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let total = sqlx::query_scalar::<_, f64>(DELIVERED_AND_IN_FLIGHT_COST_SQL)
                .bind(tenant_id)
                .bind(&operations)
                .bind(since)
                .fetch_one(&mut *tx)
                .await?;

            tx.commit().await?;
            Ok(total)
        })
    }

    fn usage_by_day(
        &self,
        tenant_id: Uuid,
        days: i32,
    ) -> BoxFuture<'_, AppResult<Vec<(String, f64)>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let rows = sqlx::query_as::<_, (String, f64)>(
                r#"
                SELECT
                    TO_CHAR(DATE(created_at), 'YYYY-MM-DD') AS day,
                    COALESCE(SUM(cost_usd), 0)::FLOAT8 AS cost
                FROM billing_events
                WHERE tenant_id = $1
                  AND created_at >= NOW() - make_interval(days => $2)
                GROUP BY DATE(created_at)
                ORDER BY DATE(created_at)
                "#,
            )
            .bind(tenant_id)
            .bind(days)
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(rows)
        })
    }

    fn usage_by_operation(
        &self,
        tenant_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Vec<(String, f64, i64)>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let rows = sqlx::query_as::<_, (String, f64, i64)>(
                r#"
                SELECT
                    operation,
                    COALESCE(SUM(cost_usd), 0)::FLOAT8 AS cost,
                    COUNT(*) AS events
                FROM billing_events
                WHERE tenant_id = $1
                GROUP BY operation
                ORDER BY cost DESC
                "#,
            )
            .bind(tenant_id)
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(rows)
        })
    }

    fn usage_totals(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<(f64, i64, i64)>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let row = sqlx::query_as::<_, (f64, i64, i64)>(
                r#"
                SELECT
                    COALESCE(SUM(cost_usd), 0)::FLOAT8,
                    COALESCE(SUM(tokens_in), 0)::BIGINT,
                    COALESCE(SUM(tokens_out), 0)::BIGINT
                FROM billing_events
                WHERE tenant_id = $1
                "#,
            )
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(row)
        })
    }

    fn inference_usage_by_day(
        &self,
        tenant_id: Uuid,
        days: i32,
    ) -> BoxFuture<'_, AppResult<Vec<InferenceUsageDay>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let rows = sqlx::query_as::<_, InferenceUsageDay>(
                r#"
                SELECT
                    TO_CHAR(DATE(created_at), 'YYYY-MM-DD') AS date,
                    COUNT(*) AS request_count,
                    COALESCE(SUM(tokens_in), 0)::BIGINT AS prompt_tokens,
                    COALESCE(SUM(tokens_out), 0)::BIGINT AS completion_tokens,
                    COALESCE(SUM(cost_usd), 0)::FLOAT8 AS cost_usd
                FROM billing_events
                WHERE tenant_id = $1
                  AND operation = 'inference'
                  AND created_at >= NOW() - make_interval(days => $2)
                GROUP BY DATE(created_at)
                ORDER BY DATE(created_at)
                "#,
            )
            .bind(tenant_id)
            .bind(days)
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(rows)
        })
    }
}
