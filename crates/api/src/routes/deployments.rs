use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
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
        .route("/models/{model_id}/deployment", get(get_deployment_status))
        .route(
            "/models/{model_id}/deployment/stream",
            get(stream_deployment_status),
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
        state.inference_backend(),
        state.config(),
        user.tenant_id,
        model_id,
    )
    .await?;
    // Billing through the unified outbox/batcher path
    state
        .record_billing_event(
            user.tenant_id,
            "deploy",
            Some(model_id),
            0,
            0,
            0,
            0.0,
            serde_json::json!({"action": "deploy"}),
        )
        .await;
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
        state.inference_backend(),
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

/// GET /api/v1/models/:model_id/deployment/stream
///
/// SSE endpoint that pushes deployment status changes.
pub async fn stream_deployment_status(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    use platform_shared::enums::DeploymentStatus;

    let initial = DeploymentService::status(state.model_repo(), user.tenant_id, model_id).await?;
    let tenant_id = user.tenant_id;

    let stream = async_stream::stream! {
        let mut last_status = initial.deployment_status;

        if let Ok(json) = serde_json::to_string(&initial) {
            yield Ok(Event::default().data(json).event("status"));
        }

        let terminal = |s: DeploymentStatus| matches!(
            s,
            DeploymentStatus::Active | DeploymentStatus::Undeployed | DeploymentStatus::Inactive
        );

        if terminal(last_status) {
            return;
        }

        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;

            match DeploymentService::status(state.model_repo(), tenant_id, model_id).await {
                Ok(status) => {
                    if status.deployment_status != last_status {
                        last_status = status.deployment_status;
                        if let Ok(json) = serde_json::to_string(&status) {
                            yield Ok(Event::default().data(json).event("status"));
                        }
                        if terminal(last_status) {
                            return;
                        }
                    } else {
                        yield Ok(Event::default().comment("heartbeat"));
                    }
                }
                Err(_) => {
                    yield Ok(Event::default().comment("heartbeat"));
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}
