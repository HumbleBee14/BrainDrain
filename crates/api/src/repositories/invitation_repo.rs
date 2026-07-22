use chrono::{DateTime, Utc};
use platform_db::models::Invitation;
use platform_db::tenant::begin_tenant_tx;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, InvitationRepository};

/// PostgreSQL implementation of the invitation repository.
///
/// Tenant-scoped queries run on the RLS pool (`db`). Token-based lookup and
/// acceptance happen before the caller is a member of the tenant, so they run
/// on the owner pool (`db_admin`) — the token is the high-entropy secret that
/// authorizes the operation.
pub struct PgInvitationRepo {
    db: PgPool,
    db_admin: PgPool,
}

impl PgInvitationRepo {
    pub fn new(db: PgPool, db_admin: PgPool) -> Self {
        Self { db, db_admin }
    }
}

impl InvitationRepository for PgInvitationRepo {
    fn create(
        &self,
        tenant_id: Uuid,
        email: &str,
        role: &str,
        token: &str,
        invited_by: &str,
        expires_at: DateTime<Utc>,
    ) -> BoxFuture<'_, AppResult<Invitation>> {
        let email = email.to_string();
        let role = role.to_string();
        let token = token.to_string();
        let invited_by = invited_by.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let invitation = sqlx::query_as::<_, Invitation>(
                r#"
                INSERT INTO invitations (tenant_id, email, role, token, invited_by, expires_at)
                VALUES ($1, $2, $3, $4, $5, $6)
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(&email)
            .bind(&role)
            .bind(&token)
            .bind(&invited_by)
            .bind(expires_at)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(invitation)
        })
    }

    fn create_with_limit(
        &self,
        tenant_id: Uuid,
        email: &str,
        role: &str,
        token: &str,
        invited_by: &str,
        expires_at: DateTime<Utc>,
        max_members: i64,
    ) -> BoxFuture<'_, AppResult<Option<Invitation>>> {
        let email = email.to_string();
        let role = role.to_string();
        let token = token.to_string();
        let invited_by = invited_by.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let invitation = sqlx::query_as::<_, Invitation>(
                r#"
                INSERT INTO invitations (tenant_id, email, role, token, invited_by, expires_at)
                SELECT $1, $2, $3, $4, $5, $6
                WHERE (SELECT COUNT(*) FROM team_members WHERE tenant_id = $1) < $7
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(&email)
            .bind(&role)
            .bind(&token)
            .bind(&invited_by)
            .bind(expires_at)
            .bind(max_members)
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(invitation)
        })
    }

    /// Looked up by token before the caller is a tenant member — owner pool.
    fn get_by_token(&self, token: &str) -> BoxFuture<'_, AppResult<Option<Invitation>>> {
        let token = token.to_string();
        Box::pin(async move {
            let invitation =
                sqlx::query_as::<_, Invitation>("SELECT * FROM invitations WHERE token = $1")
                    .bind(&token)
                    .fetch_optional(&self.db_admin)
                    .await?;

            Ok(invitation)
        })
    }

    fn list_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<Vec<Invitation>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let invitations = sqlx::query_as::<_, Invitation>(
                r#"
                SELECT * FROM invitations
                WHERE tenant_id = $1
                ORDER BY created_at DESC
                LIMIT 1000
                "#,
            )
            .bind(tenant_id)
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(invitations)
        })
    }

    /// Acceptance is authorized by the token/id and runs before membership —
    /// owner pool. The caller has already validated the token via `get_by_token`.
    fn accept(&self, id: Uuid) -> BoxFuture<'_, AppResult<Invitation>> {
        Box::pin(async move {
            let invitation = sqlx::query_as::<_, Invitation>(
                r#"
                UPDATE invitations
                SET status = 'accepted', accepted_at = now()
                WHERE id = $1
                RETURNING *
                "#,
            )
            .bind(id)
            .fetch_one(&self.db_admin)
            .await?;

            Ok(invitation)
        })
    }

    fn revoke(&self, tenant_id: Uuid, id: Uuid) -> BoxFuture<'_, AppResult<Invitation>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let invitation = sqlx::query_as::<_, Invitation>(
                r#"
                UPDATE invitations
                SET status = 'revoked'
                WHERE id = $1 AND tenant_id = $2
                RETURNING *
                "#,
            )
            .bind(id)
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;

            Ok(invitation)
        })
    }
}
