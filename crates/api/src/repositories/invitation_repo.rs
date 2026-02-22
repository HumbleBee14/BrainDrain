use chrono::{DateTime, Utc};
use platform_db::models::Invitation;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, InvitationRepository};

pub struct PgInvitationRepo {
    db: PgPool,
}

impl PgInvitationRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
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
            .fetch_one(&self.db)
            .await?;

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
            .fetch_optional(&self.db)
            .await?;

            Ok(invitation)
        })
    }

    fn get_by_token(&self, token: &str) -> BoxFuture<'_, AppResult<Option<Invitation>>> {
        let token = token.to_string();
        Box::pin(async move {
            let invitation =
                sqlx::query_as::<_, Invitation>("SELECT * FROM invitations WHERE token = $1")
                    .bind(&token)
                    .fetch_optional(&self.db)
                    .await?;

            Ok(invitation)
        })
    }

    fn list_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<Vec<Invitation>>> {
        Box::pin(async move {
            let invitations = sqlx::query_as::<_, Invitation>(
                r#"
                SELECT * FROM invitations
                WHERE tenant_id = $1
                ORDER BY created_at DESC
                LIMIT 1000
                "#,
            )
            .bind(tenant_id)
            .fetch_all(&self.db)
            .await?;

            Ok(invitations)
        })
    }

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
            .fetch_one(&self.db)
            .await?;

            Ok(invitation)
        })
    }

    fn revoke(&self, tenant_id: Uuid, id: Uuid) -> BoxFuture<'_, AppResult<Invitation>> {
        Box::pin(async move {
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
            .fetch_one(&self.db)
            .await?;

            Ok(invitation)
        })
    }
}
