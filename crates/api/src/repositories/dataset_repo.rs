use platform_db::models::Dataset;
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
}

impl PgDatasetRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
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
            .fetch_all(&self.db)
            .await?;

            Ok(datasets)
        })
    }

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Dataset>>> {
        Box::pin(async move {
            let dataset = sqlx::query_as::<_, Dataset>(
                "SELECT * FROM datasets WHERE id = $1 AND tenant_id = $2",
            )
            .bind(dataset_id)
            .bind(tenant_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(dataset)
        })
    }

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM datasets WHERE project_id = $1 AND tenant_id = $2",
            )
            .bind(project_id)
            .bind(tenant_id)
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }

    fn count_by_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: DatasetStatus,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM datasets WHERE project_id = $1 AND tenant_id = $2 AND status = $3",
            )
            .bind(project_id)
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }
}
