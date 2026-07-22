use platform_db::models::Model;
use platform_db::tenant::begin_tenant_tx;
use platform_shared::enums::DeploymentStatus;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, ModelRepository};

/// PostgreSQL implementation of the model repository.
///
/// Models are created by the Python training worker.
///
/// Tenant-scoped queries run on the RLS pool (`db`) inside a tenant-scoped
/// transaction. The global adapter-capacity accounting (`count_active_by_base_model`,
/// `claim_deployment_slot`) and the stale-deployment reaper deliberately span
/// tenants, so they run on the owner pool (`db_admin`); RLS on the RLS pool
/// would otherwise clamp their cross-tenant counts to a single tenant and break
/// the global `--max-loras` cap.
pub struct PgModelRepo {
    db: PgPool,
    db_admin: PgPool,
}

impl PgModelRepo {
    pub fn new(db: PgPool, db_admin: PgPool) -> Self {
        Self { db, db_admin }
    }
}

impl ModelRepository for PgModelRepo {
    fn get_by_id(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Model>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let model =
                sqlx::query_as::<_, Model>("SELECT * FROM models WHERE id = $1 AND tenant_id = $2")
                    .bind(model_id)
                    .bind(tenant_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            tx.commit().await?;

            Ok(model)
        })
    }

    fn list_by_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Model>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let models = sqlx::query_as::<_, Model>(
                r#"
                SELECT * FROM models
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

            Ok(models)
        })
    }

    fn count_by_project(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM models WHERE project_id = $1 AND tenant_id = $2",
            )
            .bind(project_id)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(count)
        })
    }

    fn count_by_deployment_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: DeploymentStatus,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM models WHERE project_id = $1 AND tenant_id = $2 AND deployment_status = $3",
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

    /// Global count across ALL tenants for the base-model adapter cap — owner pool.
    fn count_active_by_base_model(&self, base_model: &str) -> BoxFuture<'_, AppResult<i64>> {
        let base_model = base_model.to_string();
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM models WHERE base_model = $1 AND deployment_status = 'active'",
            )
            .bind(&base_model)
            .fetch_one(&self.db_admin)
            .await?;
            Ok(count)
        })
    }

    /// Claims a deployment slot under the GLOBAL `--max-loras` cap. Runs on the
    /// owner pool: the COUNT subquery must see active/deploying models across all
    /// tenants (RLS would clamp it to one tenant and over-admit). Tenant scoping
    /// of the claimed row is preserved by `WHERE ... tenant_id = $2`.
    fn claim_deployment_slot(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        base_model: &str,
        max_loras: i64,
    ) -> BoxFuture<'_, AppResult<bool>> {
        let base_model = base_model.to_string();
        Box::pin(async move {
            // Use a PostgreSQL advisory lock keyed on the base_model hash to
            // serialize concurrent deploy attempts for the same model family.
            // Under READ COMMITTED, two concurrent UPDATEs can evaluate the
            // COUNT(*) subquery from separate snapshots and both succeed.
            // pg_advisory_xact_lock is released automatically at transaction end.
            let mut tx = self.db_admin.begin().await?;

            // Hash the base_model string to a stable i64 for the advisory lock key
            let lock_key = base_model
                .bytes()
                .fold(0i64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as i64));

            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(lock_key)
                .execute(&mut *tx)
                .await?;

            // Now count is safe — no concurrent deploy can be between our check and claim
            let result = sqlx::query(
                r#"
                UPDATE models
                SET deployment_status = 'deploying', updated_at = NOW()
                WHERE id = $1 AND tenant_id = $2
                  AND (SELECT COUNT(*) FROM models
                       WHERE base_model = $3
                         AND deployment_status IN ('active', 'deploying')) < $4
                "#,
            )
            .bind(model_id)
            .bind(tenant_id)
            .bind(&base_model)
            .bind(max_loras)
            .execute(&mut *tx)
            .await?;

            let claimed = result.rows_affected() > 0;
            tx.commit().await?;

            Ok(claimed)
        })
    }

    /// Cross-tenant sweep of stale deploying models — owner pool.
    fn reap_stale_deployments(&self, stale_minutes: i64) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let result = sqlx::query(
                r#"
                UPDATE models
                SET deployment_status = 'undeployed', updated_at = NOW()
                WHERE deployment_status = 'deploying'
                  AND updated_at < NOW() - make_interval(mins => $1)
                "#,
            )
            .bind(stale_minutes as f64)
            .execute(&self.db_admin)
            .await?;

            let reaped = result.rows_affected() as i64;
            if reaped > 0 {
                tracing::warn!(reaped, stale_minutes, "Reaped stale deploying models");
            }
            Ok(reaped)
        })
    }

    fn update_deployment_status(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        status: DeploymentStatus,
    ) -> BoxFuture<'_, AppResult<Option<Model>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let model = sqlx::query_as::<_, Model>(
                r#"
                UPDATE models
                SET deployment_status = $3, updated_at = NOW()
                WHERE id = $1 AND tenant_id = $2
                RETURNING *
                "#,
            )
            .bind(model_id)
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(model)
        })
    }

    fn update_eval_scores(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        scores: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<bool>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let result = sqlx::query(
                r#"
                UPDATE models
                SET eval_scores = $3, updated_at = NOW()
                WHERE id = $1 AND tenant_id = $2
                "#,
            )
            .bind(model_id)
            .bind(tenant_id)
            .bind(scores)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(result.rows_affected() > 0)
        })
    }

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count =
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM models WHERE tenant_id = $1")
                    .bind(tenant_id)
                    .fetch_one(&mut *tx)
                    .await?;
            tx.commit().await?;

            Ok(count)
        })
    }

    fn count_by_tenant_deployment_status(
        &self,
        tenant_id: Uuid,
        status: DeploymentStatus,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM models WHERE tenant_id = $1 AND deployment_status = $2",
            )
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(count)
        })
    }

    fn list_versions(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        base_model: &str,
    ) -> BoxFuture<'_, AppResult<Vec<Model>>> {
        let base_model = base_model.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let models = sqlx::query_as::<_, Model>(
                r#"
                SELECT * FROM models
                WHERE project_id = $1 AND tenant_id = $2 AND base_model = $3
                ORDER BY version DESC
                LIMIT 100
                "#,
            )
            .bind(project_id)
            .bind(tenant_id)
            .bind(&base_model)
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(models)
        })
    }

    fn get_max_version(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        base_model: &str,
    ) -> BoxFuture<'_, AppResult<i32>> {
        let base_model = base_model.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let max_version = sqlx::query_scalar::<_, Option<i32>>(
                r#"
                SELECT MAX(version) FROM models
                WHERE project_id = $1 AND tenant_id = $2 AND base_model = $3
                "#,
            )
            .bind(project_id)
            .bind(tenant_id)
            .bind(&base_model)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(max_version.unwrap_or(0))
        })
    }

    fn rollback_deployment(
        &self,
        tenant_id: Uuid,
        current_id: Uuid,
        target_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Model>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;

            // Undeploy current
            sqlx::query(
                r#"
                UPDATE models
                SET deployment_status = 'undeployed', updated_at = NOW()
                WHERE id = $1 AND tenant_id = $2 AND deployment_status = 'active'
                "#,
            )
            .bind(current_id)
            .bind(tenant_id)
            .execute(&mut *tx)
            .await?;

            // Deploy target
            let model = sqlx::query_as::<_, Model>(
                r#"
                UPDATE models
                SET deployment_status = 'active', updated_at = NOW()
                WHERE id = $1 AND tenant_id = $2
                RETURNING *
                "#,
            )
            .bind(target_id)
            .bind(tenant_id)
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(model)
        })
    }
}
