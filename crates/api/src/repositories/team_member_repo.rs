use platform_db::models::TeamMember;
use platform_db::tenant::begin_tenant_tx;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, TeamMemberRepository};

pub struct PgTeamMemberRepo {
    db: PgPool,
}

impl PgTeamMemberRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl TeamMemberRepository for PgTeamMemberRepo {
    fn create(
        &self,
        tenant_id: Uuid,
        user_id: &str,
        email: &str,
        role: &str,
        invited_by: Option<&str>,
    ) -> BoxFuture<'_, AppResult<TeamMember>> {
        let user_id = user_id.to_string();
        let email = email.to_string();
        let role = role.to_string();
        let invited_by = invited_by.map(|s| s.to_string());
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            // Use ON CONFLICT DO NOTHING to handle race conditions (e.g. auto-create owner).
            // If the row already exists, RETURNING returns nothing, so we fall back to SELECT.
            let maybe = sqlx::query_as::<_, TeamMember>(
                r#"
                INSERT INTO team_members (tenant_id, user_id, email, role, invited_by)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (tenant_id, user_id) DO NOTHING
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(&user_id)
            .bind(&email)
            .bind(&role)
            .bind(invited_by.as_deref())
            .fetch_optional(&mut *tx)
            .await?;

            match maybe {
                Some(member) => {
                    tx.commit().await?;
                    Ok(member)
                }
                None => {
                    // Row already existed — fetch it
                    let member = sqlx::query_as::<_, TeamMember>(
                        "SELECT * FROM team_members WHERE tenant_id = $1 AND user_id = $2",
                    )
                    .bind(tenant_id)
                    .bind(&user_id)
                    .fetch_one(&mut *tx)
                    .await?;
                    tx.commit().await?;
                    Ok(member)
                }
            }
        })
    }

    fn get_by_user(
        &self,
        tenant_id: Uuid,
        user_id: &str,
    ) -> BoxFuture<'_, AppResult<Option<TeamMember>>> {
        let user_id = user_id.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let member = sqlx::query_as::<_, TeamMember>(
                "SELECT * FROM team_members WHERE tenant_id = $1 AND user_id = $2",
            )
            .bind(tenant_id)
            .bind(&user_id)
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(member)
        })
    }

    fn get_role(&self, tenant_id: Uuid, user_id: &str) -> BoxFuture<'_, AppResult<Option<String>>> {
        let user_id = user_id.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let role = sqlx::query_scalar::<_, String>(
                "SELECT role FROM team_members WHERE tenant_id = $1 AND user_id = $2",
            )
            .bind(tenant_id)
            .bind(&user_id)
            .fetch_optional(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(role)
        })
    }

    fn list_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<Vec<TeamMember>>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let members = sqlx::query_as::<_, TeamMember>(
                r#"
                SELECT * FROM team_members
                WHERE tenant_id = $1
                ORDER BY joined_at ASC
                LIMIT 1000
                "#,
            )
            .bind(tenant_id)
            .fetch_all(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(members)
        })
    }

    fn email_exists(&self, tenant_id: Uuid, email: &str) -> BoxFuture<'_, AppResult<bool>> {
        let email = email.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM team_members WHERE tenant_id = $1 AND email = $2)",
            )
            .bind(tenant_id)
            .bind(&email)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(exists)
        })
    }

    fn update_role(
        &self,
        tenant_id: Uuid,
        user_id: &str,
        role: &str,
    ) -> BoxFuture<'_, AppResult<TeamMember>> {
        let user_id = user_id.to_string();
        let role = role.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let member = sqlx::query_as::<_, TeamMember>(
                r#"
                UPDATE team_members
                SET role = $3
                WHERE tenant_id = $1 AND user_id = $2
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(&user_id)
            .bind(&role)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(member)
        })
    }

    fn remove(&self, tenant_id: Uuid, user_id: &str) -> BoxFuture<'_, AppResult<()>> {
        let user_id = user_id.to_string();
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            // Atomic check: only delete if the member is NOT the last owner.
            // The subquery prevents TOCTOU races where concurrent requests
            // both pass the owner check and remove the last owner.
            let result = sqlx::query(
                r#"
                DELETE FROM team_members
                WHERE tenant_id = $1 AND user_id = $2
                  AND NOT (
                    role = 'owner'
                    AND (SELECT COUNT(*) FROM team_members WHERE tenant_id = $1 AND role = 'owner') <= 1
                  )
                "#,
            )
            .bind(tenant_id)
            .bind(&user_id)
            .execute(&mut *tx)
            .await?;

            if result.rows_affected() == 0 {
                return Err(crate::error::AppError::BadRequest {
                    message: "Member not found or cannot remove the last owner".to_string(),
                });
            }

            tx.commit().await?;
            Ok(())
        })
    }

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let mut tx = begin_tenant_tx(&self.db, tenant_id).await?;
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM team_members WHERE tenant_id = $1",
            )
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;
            Ok(count)
        })
    }
}
