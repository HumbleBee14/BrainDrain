use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::tenant_settings::{LlmSettingsResponse, UpdateLlmSettingsRequest};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::tenant_settings_service::TenantSettingsService;

/// Tenant settings routes.
pub fn router() -> Router<AppState> {
    Router::new().route(
        "/settings/llm",
        get(get_llm_settings)
            .put(update_llm_settings)
            .delete(delete_llm_settings),
    )
}

/// GET /api/v1/settings/llm
#[utoipa::path(
    get,
    path = "/api/v1/settings/llm",
    tag = "Settings",
    responses(
        (status = 200, description = "LLM provider settings", body = LlmSettingsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — admin required"),
    ),
    security(("jwt" = []))
)]
pub async fn get_llm_settings(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<LlmSettingsResponse>> {
    require_role(&user, TeamRole::Admin)?;

    let settings =
        TenantSettingsService::get_llm_settings(state.tenant_repo(), user.tenant_id).await?;

    Ok(Json(settings))
}

/// PUT /api/v1/settings/llm
#[utoipa::path(
    put,
    path = "/api/v1/settings/llm",
    tag = "Settings",
    request_body = UpdateLlmSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = LlmSettingsResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — admin required"),
    ),
    security(("jwt" = []))
)]
pub async fn update_llm_settings(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(body): Json<UpdateLlmSettingsRequest>,
) -> AppResult<Json<LlmSettingsResponse>> {
    require_role(&user, TeamRole::Admin)?;

    // Audit log (never log the actual API key)
    let audit_meta = serde_json::json!({
        "provider": body.provider,
        "api_base_url": body.api_base_url,
        "model": body.model,
        "max_tokens": body.max_tokens,
        "api_key_changed": body.api_key.is_some(),
    });

    let settings =
        TenantSettingsService::update_llm_settings(state.tenant_repo(), user.tenant_id, body)
            .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "settings.llm.update",
        "tenant",
        Some(user.tenant_id),
        audit_meta,
    )
    .await;

    Ok(Json(settings))
}

/// DELETE /api/v1/settings/llm
#[utoipa::path(
    delete,
    path = "/api/v1/settings/llm",
    tag = "Settings",
    responses(
        (status = 204, description = "Settings cleared — reverts to platform defaults"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — admin required"),
    ),
    security(("jwt" = []))
)]
pub async fn delete_llm_settings(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<axum::http::StatusCode> {
    require_role(&user, TeamRole::Admin)?;

    TenantSettingsService::delete_llm_settings(state.tenant_repo(), user.tenant_id).await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "settings.llm.delete",
        "tenant",
        Some(user.tenant_id),
        serde_json::json!({}),
    )
    .await;

    Ok(axum::http::StatusCode::NO_CONTENT)
}
