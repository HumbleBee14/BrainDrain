use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};

use platform_shared::enums::TeamRole;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::notification::{
    InAppNotificationsResponse, NotificationDeliveryResponse, NotificationPreferenceResponse,
    UpdatePreferencesRequest,
};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::notification_service::NotificationService;

/// Notification routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/notifications/preferences",
            get(list_preferences).put(update_preferences),
        )
        .route("/notifications/deliveries", get(list_deliveries))
        .route("/notifications/preferences/:id/test", post(test_webhook))
        .route("/notifications/deliveries/:id/retry", post(retry_delivery))
        .route("/notifications/in-app", get(list_in_app))
        .route("/notifications/in-app/read-all", post(mark_all_in_app_read))
        .route("/notifications/in-app/:id/read", post(mark_in_app_read))
}

/// Default number of in-app notifications returned to the bell menu.
const IN_APP_LIMIT: i64 = 30;

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

    let offset = pagination.offset();
    let limit = pagination.limit();
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

/// Send a test webhook to a configured preference URL.
#[utoipa::path(
    post,
    path = "/api/v1/notifications/preferences/{id}/test",
    tag = "Notifications",
    params(("id" = Uuid, Path, description = "Preference ID")),
    responses(
        (status = 200, description = "Test delivery result", body = NotificationDeliveryResponse),
    ),
    security(("jwt" = []))
)]
pub async fn test_webhook(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(preference_id): Path<Uuid>,
) -> AppResult<Json<NotificationDeliveryResponse>> {
    require_role(&user, TeamRole::Admin)?;

    let result = NotificationService::test_webhook(
        state.notification_repo(),
        state.http_client(),
        user.tenant_id,
        preference_id,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "notification.webhook_test",
        "notification_preference",
        Some(preference_id),
        serde_json::json!({ "delivery_status": result.status }),
    )
    .await;

    Ok(Json(result))
}

/// Retry a failed notification delivery.
#[utoipa::path(
    post,
    path = "/api/v1/notifications/deliveries/{id}/retry",
    tag = "Notifications",
    params(("id" = Uuid, Path, description = "Delivery ID")),
    responses(
        (status = 200, description = "Retry result", body = NotificationDeliveryResponse),
    ),
    security(("jwt" = []))
)]
pub async fn retry_delivery(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(delivery_id): Path<Uuid>,
) -> AppResult<Json<NotificationDeliveryResponse>> {
    require_role(&user, TeamRole::Admin)?;

    let result = NotificationService::retry_delivery(
        state.notification_repo(),
        state.http_client(),
        user.tenant_id,
        delivery_id,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "notification.delivery_retry",
        "notification_delivery",
        Some(delivery_id),
        serde_json::json!({ "delivery_status": result.status }),
    )
    .await;

    Ok(Json(result))
}

/// List in-app notifications for the bell menu. Available to any team member.
#[utoipa::path(
    get,
    path = "/api/v1/notifications/in-app",
    tag = "Notifications",
    responses(
        (status = 200, description = "In-app notifications", body = InAppNotificationsResponse),
    ),
    security(("jwt" = []))
)]
pub async fn list_in_app(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<InAppNotificationsResponse>> {
    let result =
        NotificationService::list_in_app(state.notification_repo(), user.tenant_id, IN_APP_LIMIT)
            .await?;

    Ok(Json(result))
}

/// Mark a single in-app notification read.
#[utoipa::path(
    post,
    path = "/api/v1/notifications/in-app/{id}/read",
    tag = "Notifications",
    params(("id" = Uuid, Path, description = "Notification ID")),
    responses((status = 204, description = "Marked read")),
    security(("jwt" = []))
)]
pub async fn mark_in_app_read(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<axum::http::StatusCode> {
    NotificationService::mark_in_app_read(state.notification_repo(), user.tenant_id, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Mark every in-app notification read.
#[utoipa::path(
    post,
    path = "/api/v1/notifications/in-app/read-all",
    tag = "Notifications",
    responses((status = 204, description = "All marked read")),
    security(("jwt" = []))
)]
pub async fn mark_all_in_app_read(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<axum::http::StatusCode> {
    NotificationService::mark_all_in_app_read(state.notification_repo(), user.tenant_id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
