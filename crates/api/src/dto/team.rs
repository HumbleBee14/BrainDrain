use chrono::{DateTime, Utc};
use platform_shared::enums::{InvitationStatus, TeamRole};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// API response for a team member.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct TeamMemberResponse {
    pub id: Uuid,
    pub user_id: String,
    pub email: String,
    pub role: TeamRole,
    pub joined_at: DateTime<Utc>,
}

impl From<platform_db::models::TeamMember> for TeamMemberResponse {
    fn from(m: platform_db::models::TeamMember) -> Self {
        Self {
            id: m.id,
            user_id: m.user_id,
            email: m.email,
            role: m.role.parse().unwrap_or(TeamRole::Member),
            joined_at: m.joined_at,
        }
    }
}

/// API response for an invitation.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct InvitationResponse {
    pub id: Uuid,
    pub email: String,
    pub role: TeamRole,
    pub status: InvitationStatus,
    pub invited_by: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl From<platform_db::models::Invitation> for InvitationResponse {
    fn from(i: platform_db::models::Invitation) -> Self {
        Self {
            id: i.id,
            email: i.email,
            role: i.role.parse().unwrap_or(TeamRole::Member),
            status: i.status.parse().unwrap_or(InvitationStatus::Pending),
            invited_by: i.invited_by,
            expires_at: i.expires_at,
            created_at: i.created_at,
        }
    }
}

/// Request body for inviting a team member.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct InviteRequest {
    pub email: String,
    #[ts(optional)]
    pub role: Option<String>,
}

/// Request body for updating a team member's role.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct UpdateRoleRequest {
    pub role: String,
}
