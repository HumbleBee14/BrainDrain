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

/// GET /api/v1/notifications/preferences
async fn list_preferences(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<Vec<NotificationPreferenceResponse>>> {
    require_role(&user, TeamRole::Admin)?;

    let prefs =
        NotificationService::list_preferences(state.notification_repo(), user.tenant_id).await?;

    Ok(Json(prefs))
}

/// PUT /api/v1/notifications/preferences
async fn update_preferences(
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

/// GET /api/v1/notifications/deliveries
async fn list_deliveries(
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
