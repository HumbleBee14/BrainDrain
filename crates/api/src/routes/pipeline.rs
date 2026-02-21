use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::pipeline::{
    ProjectPipelineStatus, TriggerParseResponse, TriggerRefineRequest, TriggerRefineResponse,
};
use crate::error::AppResult;
use crate::services::pipeline_service::PipelineService;

/// Pipeline trigger and status routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/parse", post(trigger_parse))
        .route("/projects/{project_id}/refine", post(trigger_refine))
        .route("/projects/{project_id}/status", get(get_status))
}

/// POST /api/v1/projects/:project_id/parse
///
/// Trigger IngestWorkflow for all unparsed documents in the project.
async fn trigger_parse(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> AppResult<(StatusCode, Json<TriggerParseResponse>)> {
    let result = PipelineService::trigger_parse(
        state.db(),
        state.orchestrator(),
        user.tenant_id,
        project_id,
    )
    .await?;

    Ok((StatusCode::ACCEPTED, Json(result)))
}

/// POST /api/v1/projects/:project_id/refine
///
/// Trigger RefineWorkflow for all parsed documents in the project.
async fn trigger_refine(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(body): Json<TriggerRefineRequest>,
) -> AppResult<(StatusCode, Json<TriggerRefineResponse>)> {
    let task_type = body.task_type.as_deref().unwrap_or("question_answering");

    let result = PipelineService::trigger_refine(
        state.db(),
        state.orchestrator(),
        user.tenant_id,
        project_id,
        task_type,
        body.config,
    )
    .await?;

    Ok((StatusCode::ACCEPTED, Json(result)))
}

/// GET /api/v1/projects/:project_id/status
///
/// Get aggregate pipeline status (document and dataset counts by status).
async fn get_status(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> AppResult<Json<ProjectPipelineStatus>> {
    let status = PipelineService::get_status(state.db(), user.tenant_id, project_id).await?;

    Ok(Json(status))
}
