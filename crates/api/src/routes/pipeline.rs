use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
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
use crate::temporal::TraceContext;

/// Pipeline trigger and status routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/parse", post(trigger_parse))
        .route("/projects/{project_id}/refine", post(trigger_refine))
        .route(
            "/projects/{project_id}/full-pipeline",
            post(trigger_full_pipeline),
        )
        // NOTE: GET /projects/{id}/status is registered in routes/projects.rs
        // alongside the PUT on the same path — axum panics at startup if two
        // merged routers register the same path shape separately.
        .route(
            "/projects/{project_id}/status/stream",
            get(stream_pipeline_status),
        )
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
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
) -> AppResult<(StatusCode, Json<TriggerParseResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let trace_ctx = TraceContext::from_headers(&headers);
    let result = PipelineService::trigger_parse(
        state.document_repo(),
        state.orchestrator(),
        user.tenant_id,
        project_id,
        trace_ctx,
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
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Json(body): Json<TriggerRefineRequest>,
) -> AppResult<(StatusCode, Json<TriggerRefineResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let trace_ctx = TraceContext::from_headers(&headers);
    let task_type = body.task_type.as_deref().unwrap_or("question_answering");

    let result = PipelineService::trigger_refine(
        state.document_repo(),
        state.orchestrator(),
        user.tenant_id,
        project_id,
        task_type,
        body.config,
        trace_ctx,
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
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Json(body): Json<TriggerFullPipelineRequest>,
) -> AppResult<(StatusCode, Json<TriggerFullPipelineResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let trace_ctx = TraceContext::from_headers(&headers);
    let task_type = body.task_type.as_deref().unwrap_or("question_answering");

    let result = PipelineService::trigger_full_pipeline(
        state.document_repo(),
        state.orchestrator(),
        user.tenant_id,
        project_id,
        task_type,
        &body.base_model,
        body.training_config,
        trace_ctx,
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

/// GET /api/v1/projects/:project_id/status/stream
///
/// SSE endpoint that pushes pipeline status changes. Polls DB server-side
/// every 3s and only emits when any count field changes.
pub async fn stream_pipeline_status(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    let tenant_id = user.tenant_id;

    // Fetch initial status and send immediately
    let initial = PipelineService::get_status(
        state.document_repo(),
        state.dataset_repo(),
        state.training_job_repo(),
        state.model_repo(),
        state.evaluation_repo(),
        tenant_id,
        project_id,
    )
    .await?;

    let stream = async_stream::stream! {
        let mut last_json = serde_json::to_string(&initial).unwrap_or_default();

        if let Ok(json) = serde_json::to_string(&initial) {
            yield Ok(Event::default().data(json).event("status"));
        }

        let is_idle = |s: &ProjectPipelineStatus| -> bool {
            s.documents.parsing == 0
                && s.datasets.generating == 0
                && s.training_jobs.training == 0
                && s.training_jobs.pending == 0
        };

        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;

            match PipelineService::get_status(
                state.document_repo(),
                state.dataset_repo(),
                state.training_job_repo(),
                state.model_repo(),
                state.evaluation_repo(),
                tenant_id,
                project_id,
            )
            .await
            {
                Ok(status) => {
                    let json = serde_json::to_string(&status).unwrap_or_default();
                    if json != last_json {
                        last_json = json.clone();
                        yield Ok(Event::default().data(json).event("status"));
                    } else {
                        yield Ok(Event::default().comment("heartbeat"));
                    }
                    if is_idle(&status) {
                        return;
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
