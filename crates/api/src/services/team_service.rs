use chrono::Utc;
use platform_shared::enums::TeamRole;
use uuid::Uuid;

use crate::dto::team::{InvitationResponse, InviteRequest, TeamMemberResponse};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{InvitationRepository, TeamMemberRepository};

pub struct TeamService;

impl TeamService {
    /// List all team members for a tenant.
    pub async fn list_members(
        repo: &dyn TeamMemberRepository,
        tenant_id: Uuid,
    ) -> AppResult<Vec<TeamMemberResponse>> {
        let members = repo.list_by_tenant(tenant_id).await?;
        Ok(members.into_iter().map(Into::into).collect())
    }

    /// Invite a new team member.
    /// When max_members is provided, uses atomic INSERT...WHERE count < limit.
    pub async fn invite(
        team_repo: &dyn TeamMemberRepository,
        invite_repo: &dyn InvitationRepository,
        tenant_id: Uuid,
        invited_by: &str,
        req: InviteRequest,
        max_members: Option<i64>,
    ) -> AppResult<InvitationResponse> {
        let email = req.email.trim().to_lowercase();
        if email.is_empty() {
            return Err(AppError::BadRequest {
                message: "Email is required".to_string(),
            });
        }

        let role = req.role.unwrap_or(TeamRole::Member);
        // Block inviting as owner — owners are bootstrapped, not invited
        if role == TeamRole::Owner {
            return Err(AppError::BadRequest {
                message: "Cannot invite as owner".to_string(),
            });
        }
        let role_str = role.to_string();

        // O(1) membership check via SELECT EXISTS — no full table scan.
        if team_repo.email_exists(tenant_id, &email).await? {
            return Err(AppError::Conflict {
                message: "User is already a team member".to_string(),
            });
        }

        // Generate secure token
        let token = generate_invite_token();

        let expires_at = Utc::now() + chrono::Duration::days(7);
        let invitation = if let Some(max) = max_members {
            // Atomic limit check: INSERT ... WHERE count < max — no TOCTOU race.
            invite_repo
                .create_with_limit(
                    tenant_id, &email, &role_str, &token, invited_by, expires_at, max,
                )
                .await?
                .ok_or(AppError::Forbidden {
                    message: format!(
                        "Plan limit reached: maximum {} team_members on your current plan",
                        max
                    ),
                })?
        } else {
            invite_repo
                .create(tenant_id, &email, &role_str, &token, invited_by, expires_at)
                .await?
        };

        tracing::info!(
            tenant_id = %tenant_id,
            email = %email,
            role = %role_str,
            "Team invitation created"
        );

        Ok(invitation.into())
    }

    /// Accept an invitation by token.
    pub async fn accept_invitation(
        team_repo: &dyn TeamMemberRepository,
        invite_repo: &dyn InvitationRepository,
        token: &str,
        user_id: &str,
    ) -> AppResult<TeamMemberResponse> {
        let invitation = invite_repo
            .get_by_token(token)
            .await?
            .ok_or(AppError::NotFound {
                message: "Invitation not found".to_string(),
            })?;

        if invitation.status != "pending" {
            return Err(AppError::BadRequest {
                message: "Invitation is no longer valid".to_string(),
            });
        }

        if invitation.expires_at < Utc::now() {
            return Err(AppError::BadRequest {
                message: "Invitation has expired".to_string(),
            });
        }

        // Create team member using the email from the invitation
        let member = team_repo
            .create(
                invitation.tenant_id,
                user_id,
                &invitation.email,
                &invitation.role,
                Some(&invitation.invited_by),
            )
            .await?;

        // Mark invitation as accepted
        invite_repo.accept(invitation.id).await?;

        tracing::info!(
            tenant_id = %invitation.tenant_id,
            user_id = user_id,
            role = %invitation.role,
            "Team invitation accepted"
        );

        Ok(member.into())
    }

    /// List all invitations for a tenant.
    pub async fn list_invitations(
        repo: &dyn InvitationRepository,
        tenant_id: Uuid,
    ) -> AppResult<Vec<InvitationResponse>> {
        let invitations = repo.list_by_tenant(tenant_id).await?;
        Ok(invitations.into_iter().map(Into::into).collect())
    }

    /// Revoke a pending invitation.
    pub async fn revoke_invitation(
        repo: &dyn InvitationRepository,
        tenant_id: Uuid,
        invitation_id: Uuid,
    ) -> AppResult<InvitationResponse> {
        let invitation = repo.revoke(tenant_id, invitation_id).await?;
        Ok(invitation.into())
    }

    /// Update a team member's role.
    pub async fn update_role(
        repo: &dyn TeamMemberRepository,
        tenant_id: Uuid,
        target_user_id: &str,
        new_role: &str,
        caller_user_id: &str,
    ) -> AppResult<TeamMemberResponse> {
        // Block self-demotion — owners must ask another owner to change their role
        if target_user_id == caller_user_id {
            return Err(AppError::BadRequest {
                message: "Cannot change your own role".to_string(),
            });
        }

        // Validate role
        if !["viewer", "member", "admin", "owner"].contains(&new_role) {
            return Err(AppError::BadRequest {
                message: "Invalid role".to_string(),
            });
        }

        // Prevent removing the last owner
        if new_role != "owner" {
            let target = repo.get_by_user(tenant_id, target_user_id).await?;
            if let Some(ref t) = target
                && t.role == "owner"
            {
                let members = repo.list_by_tenant(tenant_id).await?;
                let owner_count = members.iter().filter(|m| m.role == "owner").count();
                if owner_count <= 1 {
                    return Err(AppError::BadRequest {
                        message: "Cannot demote the last owner".to_string(),
                    });
                }
            }
        }

        let member = repo
            .update_role(tenant_id, target_user_id, new_role)
            .await?;
        Ok(member.into())
    }

    /// Remove a team member.
    pub async fn remove_member(
        repo: &dyn TeamMemberRepository,
        tenant_id: Uuid,
        target_user_id: &str,
        caller_role: &str,
    ) -> AppResult<()> {
        // Prevent removing the last owner
        let target =
            repo.get_by_user(tenant_id, target_user_id)
                .await?
                .ok_or(AppError::NotFound {
                    message: "Team member not found".to_string(),
                })?;

        if target.role == "owner" {
            // Only an owner can remove another owner
            if caller_role != "owner" {
                return Err(AppError::Forbidden {
                    message: "Only an owner can remove another owner".to_string(),
                });
            }

            let members = repo.list_by_tenant(tenant_id).await?;
            let owner_count = members.iter().filter(|m| m.role == "owner").count();
            if owner_count <= 1 {
                return Err(AppError::BadRequest {
                    message: "Cannot remove the last owner".to_string(),
                });
            }
        }

        repo.remove(tenant_id, target_user_id).await?;

        tracing::info!(
            tenant_id = %tenant_id,
            target_user_id = target_user_id,
            "Team member removed"
        );

        Ok(())
    }

    // Owner auto-bootstrap is handled inline in auth.rs FromRequestParts.
    // When a user authenticates and has no team_member row, auth.rs checks
    // count_by_tenant() == 0 and auto-creates them as Owner with ON CONFLICT
    // DO NOTHING to handle race conditions.
}

fn generate_invite_token() -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.r#gen()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}
