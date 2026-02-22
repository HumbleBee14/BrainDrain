use platform_db::models::TeamMember;
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
            .fetch_optional(&self.db)
            .await?;

            match maybe {
                Some(member) => Ok(member),
                None => {
                    // Row already existed — fetch it
                    let member = sqlx::query_as::<_, TeamMember>(
                        "SELECT * FROM team_members WHERE tenant_id = $1 AND user_id = $2",
                    )
                    .bind(tenant_id)
                    .bind(&user_id)
                    .fetch_one(&self.db)
                    .await?;
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
            let member = sqlx::query_as::<_, TeamMember>(
                "SELECT * FROM team_members WHERE tenant_id = $1 AND user_id = $2",
            )
            .bind(tenant_id)
            .bind(&user_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(member)
        })
    }

    fn get_role(&self, tenant_id: Uuid, user_id: &str) -> BoxFuture<'_, AppResult<Option<String>>> {
        let user_id = user_id.to_string();
        Box::pin(async move {
            let role = sqlx::query_scalar::<_, String>(
                "SELECT role FROM team_members WHERE tenant_id = $1 AND user_id = $2",
            )
            .bind(tenant_id)
            .bind(&user_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(role)
        })
    }

    fn list_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<Vec<TeamMember>>> {
        Box::pin(async move {
            let members = sqlx::query_as::<_, TeamMember>(
                r#"
                SELECT * FROM team_members
                WHERE tenant_id = $1
                ORDER BY joined_at ASC
                LIMIT 1000
                "#,
            )
            .bind(tenant_id)
            .fetch_all(&self.db)
            .await?;

            Ok(members)
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
            .fetch_one(&self.db)
            .await?;

            Ok(member)
        })
    }

    fn remove(&self, tenant_id: Uuid, user_id: &str) -> BoxFuture<'_, AppResult<()>> {
        let user_id = user_id.to_string();
        Box::pin(async move {
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
            .execute(&self.db)
            .await?;

            if result.rows_affected() == 0 {
                return Err(crate::error::AppError::BadRequest {
                    message: "Member not found or cannot remove the last owner".to_string(),
                });
            }

            Ok(())
        })
    }

    fn count_by_tenant(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM team_members WHERE tenant_id = $1",
            )
            .bind(tenant_id)
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }
}
