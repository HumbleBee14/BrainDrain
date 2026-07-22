use platform_db::models::AuditLog;
use platform_db::tenant::begin_tenant_tx;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{AuditLogRepository, BoxFuture};

/// PostgreSQL implementation of the audit log repository.
///
/// Audit logs are append-only. All queries require `tenant_id`.
pub struct PgAuditLogRepo {
    db: PgPool,
}

impl PgAuditLogRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl AuditLogRepository for PgAuditLogRepo {
    fn create(
        &self,
        tenant_id: Uuid,
        actor_id: &str,
        action: &str,
        resource_type: &str,
        resource_id: Option<Uuid>,
        metadata: serde_json::Value,
    ) -> BoxFuture<'_, AppResult<AuditLog>> {
        let actor_id = actor_id.to_string();
        let action = action.to_string();
        let resource_type = resource_type.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let log = sqlx::query_as::<_, AuditLog>(
                r#"
                INSERT INTO audit_logs
                    (tenant_id, actor_id, action, resource_type, resource_id, metadata)
                VALUES ($1, $2, $3, $4, $5, $6)
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(&actor_id)
            .bind(&action)
            .bind(&resource_type)
            .bind(resource_id)
            .bind(metadata)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(log)
        })
    }

    fn list_by_tenant(
        &self,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<AuditLog>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let logs = sqlx::query_as::<_, AuditLog>(
                r#"
                SELECT * FROM audit_logs
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
            Ok(logs)
        })
    }

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM audit_logs WHERE tenant_id = $1",
            )
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }

    fn list_by_resource(
        &self,
        tenant_id: Uuid,
        resource_type: &str,
        resource_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<AuditLog>>> {
        let resource_type = resource_type.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let logs = sqlx::query_as::<_, AuditLog>(
                r#"
                SELECT * FROM audit_logs
                WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3
                ORDER BY created_at DESC
                LIMIT $4 OFFSET $5
                "#,
            )
            .bind(tenant_id)
            .bind(&resource_type)
            .bind(resource_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(logs)
        })
    }

    fn count_by_resource(
        &self,
        tenant_id: Uuid,
        resource_type: &str,
        resource_id: Uuid,
    ) -> BoxFuture<'_, AppResult<i64>> {
        let resource_type = resource_type.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*) FROM audit_logs
                WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3
                "#,
            )
            .bind(tenant_id)
            .bind(&resource_type)
            .bind(resource_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }

    fn list_filtered(
        &self,
        tenant_id: Uuid,
        action: Option<&str>,
        resource_type: Option<&str>,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<AuditLog>>> {
        let action = action.map(|s| s.to_string());
        let resource_type = resource_type.map(|s| s.to_string());
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let logs = sqlx::query_as::<_, AuditLog>(
                r#"
                SELECT * FROM audit_logs
                WHERE tenant_id = $1
                  AND ($2::text IS NULL OR action = $2)
                  AND ($3::text IS NULL OR resource_type = $3)
                ORDER BY created_at DESC
                LIMIT $4 OFFSET $5
                "#,
            )
            .bind(tenant_id)
            .bind(action.as_deref())
            .bind(resource_type.as_deref())
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(logs)
        })
    }

    fn count_filtered(
        &self,
        tenant_id: Uuid,
        action: Option<&str>,
        resource_type: Option<&str>,
    ) -> BoxFuture<'_, AppResult<i64>> {
        let action = action.map(|s| s.to_string());
        let resource_type = resource_type.map(|s| s.to_string());
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*) FROM audit_logs
                WHERE tenant_id = $1
                  AND ($2::text IS NULL OR action = $2)
                  AND ($3::text IS NULL OR resource_type = $3)
                "#,
            )
            .bind(tenant_id)
            .bind(action.as_deref())
            .bind(resource_type.as_deref())
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }
}
