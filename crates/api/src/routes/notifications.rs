use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::notification::{
    NotificationDeliveryResponse, NotificationPreferenceResponse, UpdatePreferencesRequest,
};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::notification_service::NotificationService;

/// Notification routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/notifications/preferences",
            get(list_preferences).put(update_preferences),
        )
        .route("/notifications/deliveries", get(list_deliveries))
}

/// List notification preferences.
#[utoipa::path(
    get,
    path = "/api/v1/notifications/preferences",
    tag = "Notifications",
    responses(
        (status = 200, description = "Notification preferences", body = Vec<NotificationPreferenceResponse>),
    ),
    security(("jwt" = []))
)]
pub async fn list_preferences(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<Vec<NotificationPreferenceResponse>>> {
    require_role(&user, TeamRole::Admin)?;

    let prefs =
        NotificationService::list_preferences(state.notification_repo(), user.tenant_id).await?;

    Ok(Json(prefs))
}

/// Update notification preferences.
#[utoipa::path(
    put,
    path = "/api/v1/notifications/preferences",
    tag = "Notifications",
    request_body = UpdatePreferencesRequest,
    responses(
        (status = 200, description = "Preferences updated", body = Vec<NotificationPreferenceResponse>),
    ),
    security(("jwt" = []))
)]
pub async fn update_preferences(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(body): Json<UpdatePreferencesRequest>,
) -> AppResult<Json<Vec<NotificationPreferenceResponse>>> {
    require_role(&user, TeamRole::Admin)?;

    let prefs = NotificationService::update_preferences(
        state.notification_repo(),
        user.tenant_id,
        body.preferences,
    )
    .await?;

    Ok(Json(prefs))
}

/// List notification deliveries.
#[utoipa::path(
    get,
    path = "/api/v1/notifications/deliveries",
    tag = "Notifications",
    params(
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
        ("limit" = Option<i64>, Query, description = "Pagination limit"),
    ),
    responses(
        (status = 200, description = "Notification deliveries", body = inline(PaginatedResponse<NotificationDeliveryResponse>)),
    ),
    security(("jwt" = []))
)]
pub async fn list_deliveries(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(pagination): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<NotificationDeliveryResponse>>> {
    require_role(&user, TeamRole::Admin)?;

    let offset = pagination.offset;
    let limit = pagination.limit.min(100);
    let repo = state.notification_repo();

    let (deliveries, total) = tokio::try_join!(
        repo.list_deliveries(user.tenant_id, offset, limit),
        repo.count_deliveries(user.tenant_id),
    )?;

    Ok(Json(PaginatedResponse {
        data: deliveries.into_iter().map(Into::into).collect(),
        total,
        offset,
        limit,
    }))
}
