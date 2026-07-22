use platform_db::models::DataGuide;
use platform_db::tenant::begin_tenant_tx;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, DataGuideRepository};

/// PostgreSQL implementation of the data guide repository.
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct PgDataGuideRepo {
    db: PgPool,
}

impl PgDataGuideRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl DataGuideRepository for PgDataGuideRepo {
    fn create(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        task_type: &str,
    ) -> BoxFuture<'_, AppResult<DataGuide>> {
        let task_type = task_type.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let guide = sqlx::query_as::<_, DataGuide>(
                r#"
                INSERT INTO data_guides (tenant_id, project_id, task_type)
                VALUES ($1, $2, $3)
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(project_id)
            .bind(task_type)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(guide)
        })
    }

    fn get(&self, tenant_id: Uuid, id: Uuid) -> BoxFuture<'_, AppResult<Option<DataGuide>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let guide = sqlx::query_as::<_, DataGuide>(
                "SELECT * FROM data_guides WHERE id = $1 AND tenant_id = $2",
            )
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(guide)
        })
    }

    fn get_for_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<DataGuide>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let guide = sqlx::query_as::<_, DataGuide>(
                r#"
                SELECT * FROM data_guides
                WHERE project_id = $1 AND tenant_id = $2
                ORDER BY created_at DESC
                LIMIT 1
                "#,
            )
            .bind(project_id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(guide)
        })
    }

    fn update_status(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        status: &str,
    ) -> BoxFuture<'_, AppResult<()>> {
        let status = status.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            sqlx::query(
                "UPDATE data_guides SET status = $3, updated_at = NOW() WHERE id = $1 AND tenant_id = $2",
            )
            .bind(id)
            .bind(tenant_id)
            .bind(status)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(())
        })
    }

    fn update_facets(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        facets: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<()>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            sqlx::query(
                "UPDATE data_guides SET facets = $3, updated_at = NOW() WHERE id = $1 AND tenant_id = $2",
            )
            .bind(id)
            .bind(tenant_id)
            .bind(facets)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(())
        })
    }

    fn apply_ratings(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        preview_samples: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<()>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            sqlx::query(
                "UPDATE data_guides SET preview_samples = $3, updated_at = NOW() WHERE id = $1 AND tenant_id = $2",
            )
            .bind(id)
            .bind(tenant_id)
            .bind(preview_samples)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(())
        })
    }

    fn update_guidance(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        guidance: &str,
        refinement_history: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<()>> {
        let guidance = guidance.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            sqlx::query(
                r#"
                UPDATE data_guides
                SET guidance = $3, refinement_history = $4, updated_at = NOW()
                WHERE id = $1 AND tenant_id = $2
                "#,
            )
            .bind(id)
            .bind(tenant_id)
            .bind(guidance)
            .bind(refinement_history)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(())
        })
    }

    fn set_dataset_id(
        &self,
        tenant_id: Uuid,
        id: Uuid,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, AppResult<()>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            sqlx::query(
                "UPDATE data_guides SET dataset_id = $3, updated_at = NOW() WHERE id = $1 AND tenant_id = $2",
            )
            .bind(id)
            .bind(tenant_id)
            .bind(dataset_id)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trait_requires_tenant_id() {
        fn assert_impl<T: DataGuideRepository>() {}
        let _ = assert_impl::<PgDataGuideRepo>;
    }
}
