use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::dashboard::{ActivityEntry, DashboardStats, UsageSummary};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::dashboard_service::DashboardService;

/// Dashboard routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/dashboard/stats", get(get_stats))
        .route("/dashboard/usage", get(get_usage))
        .route("/dashboard/activity", get(get_activity))
}

/// GET /api/v1/dashboard/stats
async fn get_stats(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<DashboardStats>> {
    require_role(&user, TeamRole::Viewer)?;

    let stats = DashboardService::get_stats(
        state.project_repo(),
        state.document_repo(),
        state.training_job_repo(),
        state.model_repo(),
        state.evaluation_repo(),
        user.tenant_id,
    )
    .await?;

    Ok(Json(stats))
}

/// GET /api/v1/dashboard/usage
async fn get_usage(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<UsageSummary>> {
    require_role(&user, TeamRole::Viewer)?;

    let usage = DashboardService::get_usage(state.billing_event_repo(), user.tenant_id).await?;

    Ok(Json(usage))
}

/// GET /api/v1/dashboard/activity
async fn get_activity(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<Vec<ActivityEntry>>> {
    require_role(&user, TeamRole::Viewer)?;

    let activity = DashboardService::get_activity(state.audit_log_repo(), user.tenant_id).await?;

    Ok(Json(activity))
}
