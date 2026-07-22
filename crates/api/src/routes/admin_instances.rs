use axum::extract::{Path, State};
use axum::routing::{delete, get, patch};
use axum::{Json, Router};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::inference_instance::{
    CreateInferenceInstanceRequest, InferenceInstanceResponse,
    UpdateInferenceInstanceLifecycleRequest,
};
use crate::error::AppResult;
use crate::rbac::require_platform_admin;
use crate::services::audit_logger::AuditLogger;
use crate::services::inference_instance_service::InferenceInstanceService;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/inference-instances",
            get(list_instances).post(register_instance),
        )
        .route(
            "/admin/inference-instances/{instance_id}/lifecycle",
            patch(update_lifecycle),
        )
        .route(
            "/admin/inference-instances/{instance_id}",
            delete(delete_instance),
        )
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/inference-instances",
    tag = "Admin",
    responses((status = 200, body = [InferenceInstanceResponse])),
    security(("jwt" = []))
)]
pub async fn list_instances(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<Vec<InferenceInstanceResponse>>> {
    require_platform_admin(&user, state.config())?;
    Ok(Json(InferenceInstanceService::list(&state).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/inference-instances",
    tag = "Admin",
    request_body = CreateInferenceInstanceRequest,
    responses((status = 201, body = InferenceInstanceResponse)),
    security(("jwt" = []))
)]
pub async fn register_instance(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(request): Json<CreateInferenceInstanceRequest>,
) -> AppResult<(axum::http::StatusCode, Json<InferenceInstanceResponse>)> {
    require_platform_admin(&user, state.config())?;
    let response = InferenceInstanceService::register(&state, request).await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "register_inference_instance",
        "inference_instance",
        Uuid::parse_str(&response.id).ok(),
        serde_json::json!({
            "backend_type": response.backend_type,
            "base_model": response.base_model,
            "base_url": response.base_url,
        }),
    )
    .await;
    Ok((axum::http::StatusCode::CREATED, Json(response)))
}

#[utoipa::path(
    patch,
    path = "/api/v1/admin/inference-instances/{instance_id}/lifecycle",
    tag = "Admin",
    request_body = UpdateInferenceInstanceLifecycleRequest,
    responses((status = 200, body = InferenceInstanceResponse)),
    security(("jwt" = []))
)]
pub async fn update_lifecycle(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(instance_id): Path<Uuid>,
    Json(request): Json<UpdateInferenceInstanceLifecycleRequest>,
) -> AppResult<Json<InferenceInstanceResponse>> {
    require_platform_admin(&user, state.config())?;
    let response = InferenceInstanceService::update_lifecycle(&state, instance_id, request).await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "update_inference_instance_lifecycle",
        "inference_instance",
        Some(instance_id),
        serde_json::json!({
            "lifecycle_state": response.lifecycle_state.to_string(),
        }),
    )
    .await;
    Ok(Json(response))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/inference-instances/{instance_id}",
    tag = "Admin",
    responses((status = 200, description = "Instance deleted")),
    security(("jwt" = []))
)]
pub async fn delete_instance(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(instance_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    require_platform_admin(&user, state.config())?;
    InferenceInstanceService::delete(&state, instance_id).await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "delete_inference_instance",
        "inference_instance",
        Some(instance_id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(serde_json::json!({"deleted": true})))
}
