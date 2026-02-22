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

/// Create a new API key for a model.
#[utoipa::path(
    post,
    path = "/api/v1/models/{model_id}/api-keys",
    tag = "API Keys",
    params(("model_id" = Uuid, Path, description = "Model ID")),
    request_body = CreateApiKeyRequest,
    responses(
        (status = 201, description = "API key created", body = CreateApiKeyResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn create_api_key(
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

/// List API keys for a model.
#[utoipa::path(
    get,
    path = "/api/v1/models/{model_id}/api-keys",
    tag = "API Keys",
    params(("model_id" = Uuid, Path, description = "Model ID")),
    responses(
        (status = 200, description = "List of API keys", body = Vec<ApiKeyResponse>),
    ),
    security(("jwt" = []))
)]
pub async fn list_api_keys(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Json<Vec<ApiKeyResponse>>> {
    let keys = ApiKeyService::list(state.api_key_repo(), user.tenant_id, model_id).await?;
    Ok(Json(keys))
}

/// Revoke an API key.
#[utoipa::path(
    post,
    path = "/api/v1/api-keys/{id}/revoke",
    tag = "API Keys",
    params(("id" = Uuid, Path, description = "API key ID")),
    responses(
        (status = 200, description = "API key revoked", body = ApiKeyResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn revoke_api_key(
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
