use platform_db::models::Tenant;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, TenantRepository};

pub struct PgTenantRepo {
    db: PgPool,
}

impl PgTenantRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl TenantRepository for PgTenantRepo {
    fn get_by_id(&self, id: Uuid) -> BoxFuture<'_, AppResult<Option<Tenant>>> {
        Box::pin(async move {
            let tenant = sqlx::query_as::<_, Tenant>("SELECT * FROM tenants WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.db)
                .await?;

            Ok(tenant)
        })
    }

    fn update_stripe_customer(&self, id: Uuid, customer_id: &str) -> BoxFuture<'_, AppResult<()>> {
        let customer_id = customer_id.to_string();
        Box::pin(async move {
            sqlx::query(
                r#"
                UPDATE tenants
                SET stripe_customer_id = $2, updated_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(id)
            .bind(&customer_id)
            .execute(&self.db)
            .await?;

            Ok(())
        })
    }

    fn update_subscription(
        &self,
        id: Uuid,
        subscription_id: &str,
        plan: &str,
        limits: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<()>> {
        let subscription_id = subscription_id.to_string();
        let plan = plan.to_string();
        Box::pin(async move {
            sqlx::query(
                r#"
                UPDATE tenants
                SET stripe_subscription_id = $2,
                    plan = $3,
                    plan_limits = $4,
                    updated_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(id)
            .bind(&subscription_id)
            .bind(&plan)
            .bind(&limits)
            .execute(&self.db)
            .await?;

            Ok(())
        })
    }

    fn get_by_stripe_customer(
        &self,
        customer_id: &str,
    ) -> BoxFuture<'_, AppResult<Option<Tenant>>> {
        let customer_id = customer_id.to_string();
        Box::pin(async move {
            let tenant =
                sqlx::query_as::<_, Tenant>("SELECT * FROM tenants WHERE stripe_customer_id = $1")
                    .bind(&customer_id)
                    .fetch_optional(&self.db)
                    .await?;

            Ok(tenant)
        })
    }

    fn get_plan_limits(&self, id: Uuid) -> BoxFuture<'_, AppResult<serde_json::Value>> {
        Box::pin(async move {
            let limits = sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT plan_limits FROM tenants WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&self.db)
            .await?;

            Ok(limits)
        })
    }

    fn sum_storage_bytes(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let total = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT (
                    (SELECT COALESCE(SUM(file_size), 0) FROM documents WHERE tenant_id = $1)
                  + (SELECT COALESCE(SUM(size_bytes), 0) FROM datasets WHERE tenant_id = $1)
                  + (SELECT COALESCE(SUM(adapter_size_bytes), 0) FROM models WHERE tenant_id = $1)
                  + (SELECT COALESCE(SUM(file_size_bytes), 0) FROM model_exports WHERE tenant_id = $1)
                )::BIGINT
                "#,
            )
            .bind(tenant_id)
            .fetch_one(&self.db)
            .await?;

            Ok(total)
        })
    }

    fn get_settings(&self, id: Uuid) -> BoxFuture<'_, AppResult<serde_json::Value>> {
        Box::pin(async move {
            let settings = sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT settings FROM tenants WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&self.db)
            .await?;

            Ok(settings)
        })
    }

    fn update_settings(
        &self,
        id: Uuid,
        settings: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<()>> {
        Box::pin(async move {
            // JSONB || operator: shallow merge at top level.
            // Replaces the "llm" key (or any other top-level key) while
            // preserving other top-level keys in the settings object.
            sqlx::query(
                r#"
                UPDATE tenants
                SET settings = settings || $2, updated_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(id)
            .bind(&settings)
            .execute(&self.db)
            .await?;

            Ok(())
        })
    }

    fn delete(&self, id: Uuid) -> BoxFuture<'_, AppResult<bool>> {
        Box::pin(async move {
            let result = sqlx::query("DELETE FROM tenants WHERE id = $1")
                .bind(id)
                .execute(&self.db)
                .await?;

            Ok(result.rows_affected() > 0)
        })
    }
}
