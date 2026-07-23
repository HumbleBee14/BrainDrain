use platform_db::models::InferenceSample;
use platform_db::tenant::begin_tenant_tx;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, InferenceSampleRepository};

/// PostgreSQL implementation of the inference sample repository (data flywheel).
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct PgInferenceSampleRepo {
    db: PgPool,
}

impl PgInferenceSampleRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl InferenceSampleRepository for PgInferenceSampleRepo {
    fn insert(
        &self,
        tenant_id: Uuid,
        sample_id: Uuid,
        model_id: Uuid,
        api_key_id: Option<Uuid>,
        messages: serde_json::Value,
        response: &str,
    ) -> BoxFuture<'_, AppResult<()>> {
        let response = response.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            sqlx::query(
                r#"
                INSERT INTO inference_samples (id, tenant_id, model_id, api_key_id, messages, response)
                VALUES ($1, $2, $3, $4, $5, $6)
                "#,
            )
            .bind(sample_id)
            .bind(tenant_id)
            .bind(model_id)
            .bind(api_key_id)
            .bind(&messages)
            .bind(&response)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(())
        })
    }

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        sample_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<InferenceSample>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let sample = sqlx::query_as::<_, InferenceSample>(
                "SELECT * FROM inference_samples WHERE id = $1 AND tenant_id = $2",
            )
            .bind(sample_id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(sample)
        })
    }

    fn list_by_model(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        rating: Option<String>,
        unrated_only: bool,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<InferenceSample>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let samples = sqlx::query_as::<_, InferenceSample>(
                r#"
                SELECT * FROM inference_samples
                WHERE model_id = $1 AND tenant_id = $2
                  AND ($3::varchar IS NULL OR rating = $3)
                  AND (NOT $4 OR rating IS NULL)
                ORDER BY created_at DESC
                LIMIT $5 OFFSET $6
                "#,
            )
            .bind(model_id)
            .bind(tenant_id)
            .bind(&rating)
            .bind(unrated_only)
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(samples)
        })
    }

    fn count_by_model(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        rating: Option<String>,
        unrated_only: bool,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*) FROM inference_samples
                WHERE model_id = $1 AND tenant_id = $2
                  AND ($3::varchar IS NULL OR rating = $3)
                  AND (NOT $4 OR rating IS NULL)
                "#,
            )
            .bind(model_id)
            .bind(tenant_id)
            .bind(&rating)
            .bind(unrated_only)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }

    fn set_rating(
        &self,
        tenant_id: Uuid,
        sample_id: Uuid,
        rating: &str,
        comment: Option<String>,
    ) -> BoxFuture<'_, AppResult<bool>> {
        let rating = rating.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let result = sqlx::query(
                r#"
                UPDATE inference_samples
                SET rating = $3, rating_comment = $4
                WHERE id = $1 AND tenant_id = $2
                "#,
            )
            .bind(sample_id)
            .bind(tenant_id)
            .bind(&rating)
            .bind(&comment)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(result.rows_affected() > 0)
        })
    }
}
