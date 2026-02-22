use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use uuid::Uuid;

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::api_key::{ApiKeyResponse, CreateApiKeyRequest, CreateApiKeyResponse};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::api_key_service::ApiKeyService;
use crate::services::audit_logger::AuditLogger;

/// API key routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/models/{model_id}/api-keys",
            post(create_api_key).get(list_api_keys),
        )
        .route("/api-keys/{id}/revoke", post(revoke_api_key))
}

/// POST /api/v1/models/:model_id/api-keys
async fn create_api_key(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
    Json(body): Json<CreateApiKeyRequest>,
) -> AppResult<(StatusCode, Json<CreateApiKeyResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let result = ApiKeyService::create(
        state.api_key_repo(),
        state.model_repo(),
        user.tenant_id,
        model_id,
        body,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "create",
        "api_key",
        result.id.parse().ok(),
        serde_json::json!({"model_id": model_id.to_string(), "key_prefix": result.key_prefix}),
    )
    .await;

    Ok((StatusCode::CREATED, Json(result)))
}

/// GET /api/v1/models/:model_id/api-keys
async fn list_api_keys(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Json<Vec<ApiKeyResponse>>> {
    let keys = ApiKeyService::list(state.api_key_repo(), user.tenant_id, model_id).await?;
    Ok(Json(keys))
}

/// POST /api/v1/api-keys/:id/revoke
async fn revoke_api_key(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ApiKeyResponse>> {
    require_role(&user, TeamRole::Member)?;
    let key = ApiKeyService::revoke(state.api_key_repo(), user.tenant_id, id).await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "revoke",
        "api_key",
        Some(id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(key))
}
