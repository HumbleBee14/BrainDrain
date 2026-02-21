use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::model::ModelResponse;
use crate::error::AppResult;
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
async fn deploy_model(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Json<ModelResponse>> {
    let result = DeploymentService::deploy(
        state.model_repo(),
        state.billing_event_repo(),
        state.config(),
        user.tenant_id,
        model_id,
    )
    .await?;
    Ok(Json(result))
}

/// POST /api/v1/models/:model_id/undeploy
async fn undeploy_model(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Json<ModelResponse>> {
    let result = DeploymentService::undeploy(
        state.model_repo(),
        state.config(),
        user.tenant_id,
        model_id,
    )
    .await?;
    Ok(Json(result))
}

/// GET /api/v1/models/:model_id/deployment
async fn get_deployment_status(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Json<DeploymentStatusResponse>> {
    let status =
        DeploymentService::status(state.model_repo(), user.tenant_id, model_id).await?;
    Ok(Json(status))
}
