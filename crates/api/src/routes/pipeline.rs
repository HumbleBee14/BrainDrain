use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::pipeline::{
    ProjectPipelineStatus, TriggerFullPipelineRequest, TriggerFullPipelineResponse,
    TriggerParseResponse, TriggerRefineRequest, TriggerRefineResponse,
};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::pipeline_service::PipelineService;

/// Pipeline trigger and status routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/parse", post(trigger_parse))
        .route("/projects/{project_id}/refine", post(trigger_refine))
        .route(
            "/projects/{project_id}/full-pipeline",
            post(trigger_full_pipeline),
        )
        .route("/projects/{project_id}/status", get(get_status))
}

/// Trigger IngestWorkflow for all unparsed documents in the project.
#[utoipa::path(
    post,
    path = "/api/v1/projects/{project_id}/parse",
    tag = "Pipeline",
    params(("project_id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 202, description = "Parse triggered", body = TriggerParseResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn trigger_parse(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> AppResult<(StatusCode, Json<TriggerParseResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let result = PipelineService::trigger_parse(
        state.document_repo(),
        state.orchestrator(),
        user.tenant_id,
        project_id,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "trigger_parse",
        "project",
        Some(project_id),
        serde_json::json!({"document_count": result.document_count}),
    )
    .await;

    Ok((StatusCode::ACCEPTED, Json(result)))
}

/// Trigger RefineWorkflow for all parsed documents in the project.
#[utoipa::path(
    post,
    path = "/api/v1/projects/{project_id}/refine",
    tag = "Pipeline",
    params(("project_id" = Uuid, Path, description = "Project ID")),
    request_body = TriggerRefineRequest,
    responses(
        (status = 202, description = "Refine triggered", body = TriggerRefineResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn trigger_refine(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(body): Json<TriggerRefineRequest>,
) -> AppResult<(StatusCode, Json<TriggerRefineResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let task_type = body.task_type.as_deref().unwrap_or("question_answering");

    let result = PipelineService::trigger_refine(
        state.document_repo(),
        state.orchestrator(),
        user.tenant_id,
        project_id,
        task_type,
        body.config,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "trigger_refine",
        "project",
        Some(project_id),
        serde_json::json!({"task_type": task_type, "document_count": result.document_count}),
    )
    .await;

    Ok((StatusCode::ACCEPTED, Json(result)))
}

/// Trigger the full pipeline: ingest → refine → train → evaluate → deploy.
#[utoipa::path(
    post,
    path = "/api/v1/projects/{project_id}/full-pipeline",
    tag = "Pipeline",
    params(("project_id" = Uuid, Path, description = "Project ID")),
    request_body = TriggerFullPipelineRequest,
    responses(
        (status = 202, description = "Full pipeline triggered", body = TriggerFullPipelineResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn trigger_full_pipeline(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(body): Json<TriggerFullPipelineRequest>,
) -> AppResult<(StatusCode, Json<TriggerFullPipelineResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let task_type = body.task_type.as_deref().unwrap_or("question_answering");

    let result = PipelineService::trigger_full_pipeline(
        state.document_repo(),
        state.orchestrator(),
        user.tenant_id,
        project_id,
        task_type,
        &body.base_model,
        body.training_config,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "trigger_full_pipeline",
        "project",
        Some(project_id),
        serde_json::json!({
            "base_model": body.base_model,
            "document_count": result.document_count,
        }),
    )
    .await;

    Ok((StatusCode::ACCEPTED, Json(result)))
}

/// Get aggregate pipeline status (document and dataset counts by status).
#[utoipa::path(
    get,
    path = "/api/v1/projects/{project_id}/status",
    tag = "Pipeline",
    params(("project_id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Pipeline status", body = ProjectPipelineStatus),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn get_status(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> AppResult<Json<ProjectPipelineStatus>> {
    let status = PipelineService::get_status(
        state.document_repo(),
        state.dataset_repo(),
        state.training_job_repo(),
        state.model_repo(),
        state.evaluation_repo(),
        user.tenant_id,
        project_id,
    )
    .await?;

    Ok(Json(status))
}
