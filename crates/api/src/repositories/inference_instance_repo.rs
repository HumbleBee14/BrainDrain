use platform_db::models::InferenceInstance;
use platform_shared::enums::{InferenceInstanceHealthStatus, InferenceInstanceLifecycleState};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, InferenceInstanceRepository};

/// PostgreSQL implementation of the inference instance repository.
pub struct PgInferenceInstanceRepo {
    db: PgPool,
}

impl PgInferenceInstanceRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl InferenceInstanceRepository for PgInferenceInstanceRepo {
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
    ) -> BoxFuture<'_, AppResult<InferenceInstance>> {
        let name = name.to_string();
        let base_url = base_url.to_string();
        let backend_type = backend_type.to_string();
        let gpu_class = gpu_class.map(ToOwned::to_owned);
        let base_model = base_model.to_string();
        Box::pin(async move {
            let instance = sqlx::query_as::<_, InferenceInstance>(
                r#"
                INSERT INTO inference_instances (
                    name, base_url, backend_type, gpu_class, base_model, max_adapters,
                    health_status, lifecycle_state, metadata
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                RETURNING *
                "#,
            )
            .bind(&name)
            .bind(&base_url)
            .bind(&backend_type)
            .bind(gpu_class)
            .bind(&base_model)
            .bind(max_adapters)
            .bind(health_status.to_string())
            .bind(lifecycle_state.to_string())
            .bind(metadata)
            .fetch_one(&self.db)
            .await?;

            Ok(instance)
        })
    }

    fn get_by_id(&self, id: Uuid) -> BoxFuture<'_, AppResult<Option<InferenceInstance>>> {
        Box::pin(async move {
            let instance = sqlx::query_as::<_, InferenceInstance>(
                "SELECT * FROM inference_instances WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.db)
            .await?;

            Ok(instance)
        })
    }

    fn list(&self) -> BoxFuture<'_, AppResult<Vec<InferenceInstance>>> {
        Box::pin(async move {
            let instances = sqlx::query_as::<_, InferenceInstance>(
                r#"
                SELECT * FROM inference_instances
                ORDER BY base_model ASC, backend_type ASC, created_at ASC
                "#,
            )
            .fetch_all(&self.db)
            .await?;

            Ok(instances)
        })
    }

    fn claim_slot(
        &self,
        backend_type: &str,
        base_model: &str,
    ) -> BoxFuture<'_, AppResult<Option<InferenceInstance>>> {
        let backend_type = backend_type.to_string();
        let base_model = base_model.to_string();
        Box::pin(async move {
            let instance = sqlx::query_as::<_, InferenceInstance>(
                r#"
                WITH candidate AS (
                    SELECT id
                    FROM inference_instances
                    WHERE backend_type = $1
                      AND base_model = $2
                      AND lifecycle_state = 'ready'
                      AND health_status = 'healthy'
                      AND active_adapter_count < max_adapters
                    ORDER BY active_adapter_count ASC, created_at ASC
                    FOR UPDATE SKIP LOCKED
                    LIMIT 1
                )
                UPDATE inference_instances AS i
                SET active_adapter_count = i.active_adapter_count + 1,
                    updated_at = NOW()
                FROM candidate
                WHERE i.id = candidate.id
                RETURNING i.*
                "#,
            )
            .bind(&backend_type)
            .bind(&base_model)
            .fetch_optional(&self.db)
            .await?;

            Ok(instance)
        })
    }

    fn release_slot(&self, id: Uuid) -> BoxFuture<'_, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                r#"
                UPDATE inference_instances
                SET active_adapter_count = GREATEST(active_adapter_count - 1, 0),
                    updated_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(id)
            .execute(&self.db)
            .await?;

            Ok(())
        })
    }

    fn update_health(
        &self,
        id: Uuid,
        health_status: InferenceInstanceHealthStatus,
    ) -> BoxFuture<'_, AppResult<Option<InferenceInstance>>> {
        let is_healthy = matches!(health_status, InferenceInstanceHealthStatus::Healthy);
        Box::pin(async move {
            let instance = sqlx::query_as::<_, InferenceInstance>(
                r#"
                UPDATE inference_instances
                SET health_status = $2,
                    last_health_check_at = NOW(),
                    last_healthy_at = CASE WHEN $3 THEN NOW() ELSE last_healthy_at END,
                    updated_at = NOW()
                WHERE id = $1
                RETURNING *
                "#,
            )
            .bind(id)
            .bind(health_status.to_string())
            .bind(is_healthy)
            .fetch_optional(&self.db)
            .await?;

            Ok(instance)
        })
    }

    fn update_lifecycle_state(
        &self,
        id: Uuid,
        lifecycle_state: InferenceInstanceLifecycleState,
    ) -> BoxFuture<'_, AppResult<Option<InferenceInstance>>> {
        Box::pin(async move {
            let instance = sqlx::query_as::<_, InferenceInstance>(
                r#"
                UPDATE inference_instances
                SET lifecycle_state = $2, updated_at = NOW()
                WHERE id = $1
                RETURNING *
                "#,
            )
            .bind(id)
            .bind(lifecycle_state.to_string())
            .fetch_optional(&self.db)
            .await?;

            Ok(instance)
        })
    }

    fn retire_if_empty(&self, id: Uuid) -> BoxFuture<'_, AppResult<Option<InferenceInstance>>> {
        Box::pin(async move {
            let row = sqlx::query_as::<_, InferenceInstance>(
                r#"
                UPDATE inference_instances
                SET lifecycle_state = 'retired', updated_at = NOW()
                WHERE id = $1 AND active_adapter_count = 0
                RETURNING *
                "#,
            )
            .bind(id)
            .fetch_optional(&self.db)
            .await?;

            Ok(row)
        })
    }

    fn delete_if_empty(&self, id: Uuid) -> BoxFuture<'_, AppResult<bool>> {
        Box::pin(async move {
            let result = sqlx::query(
                "DELETE FROM inference_instances WHERE id = $1 AND active_adapter_count = 0",
            )
            .bind(id)
            .execute(&self.db)
            .await?;

            Ok(result.rows_affected() > 0)
        })
    }

    fn list_for_healthcheck(&self) -> BoxFuture<'_, AppResult<Vec<InferenceInstance>>> {
        Box::pin(async move {
            let instances = sqlx::query_as::<_, InferenceInstance>(
                r#"
                SELECT * FROM inference_instances
                WHERE lifecycle_state IN ('ready', 'draining')
                ORDER BY created_at ASC
                "#,
            )
            .fetch_all(&self.db)
            .await?;

            Ok(instances)
        })
    }

    fn reconcile_adapter_counts(&self) -> BoxFuture<'_, AppResult<u64>> {
        Box::pin(async move {
            let result = sqlx::query(
                r#"
                UPDATE inference_instances AS i
                SET active_adapter_count = counts.bound_models,
                    updated_at = NOW()
                FROM (
                    SELECT
                        i2.id,
                        COALESCE((
                            SELECT COUNT(*)
                            FROM models m
                            WHERE m.inference_instance_id = i2.id
                              AND m.deployment_status IN ('active', 'deploying')
                        ), 0)::INTEGER AS bound_models
                    FROM inference_instances i2
                ) AS counts
                WHERE i.id = counts.id
                  AND i.active_adapter_count <> counts.bound_models
                "#,
            )
            .execute(&self.db)
            .await?;

            Ok(result.rows_affected())
        })
    }
}
