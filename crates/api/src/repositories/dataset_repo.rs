use platform_db::models::Dataset;
use platform_db::tenant::begin_tenant_tx;
use platform_shared::enums::DatasetStatus;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, DatasetRepository};

/// PostgreSQL implementation of the dataset repository.
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct PgDatasetRepo {
    db: PgPool,
    /// Cross-tenant sweeps have no tenant_id to satisfy RLS, so they run on the
    /// owner pool.
    db_admin: PgPool,
}

impl PgDatasetRepo {
    pub fn new(db: PgPool, db_admin: PgPool) -> Self {
        Self { db, db_admin }
    }
}

impl DatasetRepository for PgDatasetRepo {
    fn list_by_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Dataset>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let datasets = sqlx::query_as::<_, Dataset>(
                r#"
                SELECT * FROM datasets
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
            Ok(datasets)
        })
    }

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Dataset>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let dataset = sqlx::query_as::<_, Dataset>(
                "SELECT * FROM datasets WHERE id = $1 AND tenant_id = $2",
            )
            .bind(dataset_id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(dataset)
        })
    }

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM datasets WHERE project_id = $1 AND tenant_id = $2",
            )
            .bind(project_id)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }

    fn sum_pair_count(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let total = sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(SUM(pair_count), 0)::BIGINT FROM datasets WHERE tenant_id = $1",
            )
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(total)
        })
    }

    fn count_by_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: DatasetStatus,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM datasets WHERE project_id = $1 AND tenant_id = $2 AND status = $3",
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

    fn update_status(
        &self,
        tenant_id: Uuid,
        dataset_id: Uuid,
        status: DatasetStatus,
    ) -> BoxFuture<'_, AppResult<Option<Dataset>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let dataset = sqlx::query_as::<_, Dataset>(
                r#"
                UPDATE datasets SET status = $3, updated_at = NOW()
                WHERE id = $1 AND tenant_id = $2
                RETURNING *
                "#,
            )
            .bind(dataset_id)
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(dataset)
        })
    }

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
    ) -> BoxFuture<'_, AppResult<Dataset>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let dataset = sqlx::query_as::<_, Dataset>(
                r#"
                INSERT INTO datasets
                    (id, tenant_id, project_id, name, format, storage_path,
                     status, pair_count, stats, config, size_bytes)
                VALUES ($1, $2, $3, $4, 'chatml', $5, $6, $7, $8, '{}'::jsonb, $9)
                RETURNING *
                "#,
            )
            .bind(dataset_id)
            .bind(tenant_id)
            .bind(project_id)
            .bind(name)
            .bind(storage_path)
            .bind(DatasetStatus::ReviewPending.to_string())
            .bind(pair_count)
            .bind(stats)
            .bind(size_bytes)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(dataset)
        })
    }

    fn create_generating(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        dataset_id: Uuid,
        name: String,
        config: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<Dataset>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let dataset = sqlx::query_as::<_, Dataset>(
                r#"
                INSERT INTO datasets
                    (id, tenant_id, project_id, name, format, status, pair_count, config)
                VALUES ($1, $2, $3, $4, 'chatml', $5, 0, $6)
                RETURNING *
                "#,
            )
            .bind(dataset_id)
            .bind(tenant_id)
            .bind(project_id)
            .bind(name)
            .bind(DatasetStatus::Generating.to_string())
            .bind(config)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(dataset)
        })
    }

    fn mark_failed(
        &self,
        tenant_id: Uuid,
        dataset_id: Uuid,
        error: String,
    ) -> BoxFuture<'_, AppResult<Option<Dataset>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let dataset = sqlx::query_as::<_, Dataset>(
                r#"
                UPDATE datasets
                SET status = $3, error = $4, updated_at = now()
                WHERE id = $1 AND tenant_id = $2
                RETURNING *
                "#,
            )
            .bind(dataset_id)
            .bind(tenant_id)
            .bind(DatasetStatus::Failed.to_string())
            .bind(error)
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(dataset)
        })
    }

    fn reap_stale_generating(&self, stale_minutes: i64) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let result = sqlx::query(
                r#"
                UPDATE datasets
                SET status = 'failed',
                    error = 'Generation stopped unexpectedly and did not report a result',
                    updated_at = NOW()
                WHERE status = 'generating'
                  AND updated_at < NOW() - make_interval(mins => $1)
                "#,
            )
            .bind(stale_minutes as f64)
            .execute(&self.db_admin)
            .await?;

            let reaped = result.rows_affected() as i64;
            if reaped > 0 {
                tracing::warn!(reaped, stale_minutes, "Reaped stale generating datasets");
            }
            Ok(reaped)
        })
    }
}
