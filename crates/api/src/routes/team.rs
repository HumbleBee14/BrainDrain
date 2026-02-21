use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use platform_shared::enums::TeamRole;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::team::{InvitationResponse, InviteRequest, TeamMemberResponse, UpdateRoleRequest};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::plan_service::PlanService;
use crate::services::team_service::TeamService;

/// Team management routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/team/members", get(list_members))
        .route("/team/invitations", post(create_invitation))
        .route("/team/invitations", get(list_invitations))
        .route("/team/invitations/{id}/revoke", post(revoke_invitation))
        .route("/team/members/{user_id}/role", put(update_role))
        .route("/team/members/{user_id}", delete(remove_member))
}

/// Route for accepting invitations (mounted separately — requires auth to
/// prevent impersonation, but no RBAC role check since the user may not yet
/// be a team member).
pub fn public_router() -> Router<AppState> {
    Router::new().route("/invitations/{token}/accept", post(accept_invitation))
}

async fn list_members(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<Vec<TeamMemberResponse>>> {
    require_role(&user, TeamRole::Viewer)?;
    let members = TeamService::list_members(state.team_member_repo(), user.tenant_id).await?;
    Ok(Json(members))
}

async fn create_invitation(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(body): Json<InviteRequest>,
) -> AppResult<(StatusCode, Json<InvitationResponse>)> {
    require_role(&user, TeamRole::Admin)?;
    let current_members = state.team_member_repo().count_by_tenant(user.tenant_id).await?;
    PlanService::check_limit(state.tenant_repo(), user.tenant_id, "team_members", current_members).await?;
    let invitation = TeamService::invite(
        state.team_member_repo(),
        state.invitation_repo(),
        user.tenant_id,
        &user.user_id,
        body,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "invite",
        "team_member",
        Some(invitation.id),
        serde_json::json!({"email": invitation.email, "role": invitation.role.to_string()}),
    )
    .await;

    Ok((StatusCode::CREATED, Json(invitation)))
}

async fn list_invitations(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<Vec<InvitationResponse>>> {
    require_role(&user, TeamRole::Admin)?;
    let invitations =
        TeamService::list_invitations(state.invitation_repo(), user.tenant_id).await?;
    Ok(Json(invitations))
}

async fn revoke_invitation(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<InvitationResponse>> {
    require_role(&user, TeamRole::Admin)?;
    let invitation =
        TeamService::revoke_invitation(state.invitation_repo(), user.tenant_id, id).await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "revoke_invitation",
        "invitation",
        Some(id),
        serde_json::json!({}),
    )
    .await;

    Ok(Json(invitation))
}

async fn accept_invitation(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(token): Path<String>,
) -> AppResult<(StatusCode, Json<TeamMemberResponse>)> {
    // user_id comes from the auth token, not the request body —
    // prevents impersonation attacks where an attacker accepts
    // an invitation as a different user.
    let member = TeamService::accept_invitation(
        state.team_member_repo(),
        state.invitation_repo(),
        &token,
        &user.user_id,
        "", // email not available from JWT — updated on next profile sync
    )
    .await?;

    Ok((StatusCode::CREATED, Json(member)))
}

async fn update_role(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(target_user_id): Path<String>,
    Json(body): Json<UpdateRoleRequest>,
) -> AppResult<Json<TeamMemberResponse>> {
    require_role(&user, TeamRole::Owner)?;
    let role_str = body.role.to_string();
    let member = TeamService::update_role(
        state.team_member_repo(),
        user.tenant_id,
        &target_user_id,
        &role_str,
        &user.user_id,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "update_role",
        "team_member",
        None,
        serde_json::json!({"target_user_id": target_user_id, "new_role": role_str}),
    )
    .await;

    Ok(Json(member))
}

async fn remove_member(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(target_user_id): Path<String>,
) -> AppResult<StatusCode> {
    require_role(&user, TeamRole::Admin)?;
    TeamService::remove_member(state.team_member_repo(), user.tenant_id, &target_user_id, &user.role.to_string()).await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "remove_member",
        "team_member",
        None,
        serde_json::json!({"target_user_id": target_user_id}),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

