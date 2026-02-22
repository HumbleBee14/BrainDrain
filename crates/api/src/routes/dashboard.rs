use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::dashboard::{ActivityEntry, DashboardStats, UsageSummary};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::repositories::billing_event_repo::InferenceUsageDay;
use crate::services::dashboard_service::DashboardService;

/// Dashboard routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/dashboard/stats", get(get_stats))
        .route("/dashboard/usage", get(get_usage))
        .route("/dashboard/activity", get(get_activity))
        .route("/dashboard/inference-usage", get(get_inference_usage))
}

/// GET /api/v1/dashboard/stats
#[utoipa::path(
    get,
    path = "/api/v1/dashboard/stats",
    tag = "Dashboard",
    responses(
        (status = 200, description = "Dashboard statistics", body = DashboardStats),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn get_stats(
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
        state.redis(),
        user.tenant_id,
    )
    .await?;

    Ok(Json(stats))
}

/// GET /api/v1/dashboard/usage
#[utoipa::path(
    get,
    path = "/api/v1/dashboard/usage",
    tag = "Dashboard",
    responses(
        (status = 200, description = "Usage summary", body = UsageSummary),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn get_usage(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<UsageSummary>> {
    require_role(&user, TeamRole::Viewer)?;

    let usage =
        DashboardService::get_usage(state.billing_event_repo(), state.redis(), user.tenant_id)
            .await?;

    Ok(Json(usage))
}

/// GET /api/v1/dashboard/inference-usage
///
/// Cached in Redis for 30s per tenant (same as stats).
#[utoipa::path(
    get,
    path = "/api/v1/dashboard/inference-usage",
    tag = "Dashboard",
    responses(
        (status = 200, description = "Daily inference usage breakdown", body = Vec<InferenceUsageDay>),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn get_inference_usage(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<Vec<crate::repositories::billing_event_repo::InferenceUsageDay>>> {
    use redis::AsyncCommands;

    require_role(&user, TeamRole::Viewer)?;

    let cache_key = format!("dashboard_inference_usage:{}", user.tenant_id);
    let mut redis = state.redis();

    // Try cache first
    if let Ok(Some(json_str)) = redis.get::<_, Option<String>>(&cache_key).await
        && let Ok(data) = serde_json::from_str::<
            Vec<crate::repositories::billing_event_repo::InferenceUsageDay>,
        >(&json_str)
    {
        return Ok(Json(data));
    }

    let data = state
        .billing_event_repo()
        .inference_usage_by_day(user.tenant_id, 30)
        .await?;

    // Cache for 30s (best-effort)
    if let Ok(json_str) = serde_json::to_string(&data) {
        let _: Result<(), _> = redis.set_ex(&cache_key, json_str, 30).await;
    }

    Ok(Json(data))
}

/// GET /api/v1/dashboard/activity
#[utoipa::path(
    get,
    path = "/api/v1/dashboard/activity",
    tag = "Dashboard",
    responses(
        (status = 200, description = "Recent activity entries", body = Vec<ActivityEntry>),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn get_activity(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<Vec<ActivityEntry>>> {
    require_role(&user, TeamRole::Viewer)?;

    let activity = DashboardService::get_activity(state.audit_log_repo(), user.tenant_id).await?;

    Ok(Json(activity))
}
