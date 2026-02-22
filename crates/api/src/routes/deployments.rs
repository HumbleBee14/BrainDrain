use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use uuid::Uuid;

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::model::ModelResponse;
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::deployment_service::{DeploymentService, DeploymentStatusResponse};

/// Deployment management routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/models/{model_id}/deploy", post(deploy_model))
        .route("/models/{model_id}/undeploy", post(undeploy_model))
        .route(
            "/models/{model_id}/deployment",
            axum::routing::get(get_deployment_status),
        )
}

/// POST /api/v1/models/:model_id/deploy
#[utoipa::path(
    post,
    path = "/api/v1/models/{model_id}/deploy",
    tag = "Deployments",
    params(
        ("model_id" = Uuid, Path, description = "Model ID")
    ),
    responses(
        (status = 200, description = "Model deployed", body = ModelResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn deploy_model(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Json<ModelResponse>> {
    require_role(&user, TeamRole::Member)?;
    let result = DeploymentService::deploy(
        state.model_repo(),
        state.billing_event_repo(),
        state.http_client(),
        state.vllm_circuit_breaker(),
        state.config(),
        user.tenant_id,
        model_id,
    )
    .await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "deploy",
        "model",
        Some(model_id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(result))
}

/// POST /api/v1/models/:model_id/undeploy
#[utoipa::path(
    post,
    path = "/api/v1/models/{model_id}/undeploy",
    tag = "Deployments",
    params(
        ("model_id" = Uuid, Path, description = "Model ID")
    ),
    responses(
        (status = 200, description = "Model undeployed", body = ModelResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn undeploy_model(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Json<ModelResponse>> {
    require_role(&user, TeamRole::Member)?;
    let result = DeploymentService::undeploy(
        state.model_repo(),
        state.http_client(),
        state.config(),
        user.tenant_id,
        model_id,
    )
    .await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "undeploy",
        "model",
        Some(model_id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(result))
}

/// GET /api/v1/models/:model_id/deployment
#[utoipa::path(
    get,
    path = "/api/v1/models/{model_id}/deployment",
    tag = "Deployments",
    params(
        ("model_id" = Uuid, Path, description = "Model ID")
    ),
    responses(
        (status = 200, description = "Deployment status", body = DeploymentStatusResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn get_deployment_status(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Json<DeploymentStatusResponse>> {
    let status = DeploymentService::status(state.model_repo(), user.tenant_id, model_id).await?;
    Ok(Json(status))
}
