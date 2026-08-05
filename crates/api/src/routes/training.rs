use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::model::{ModelResponse, RollbackModelRequest};
use crate::dto::training_job::{
    CostEstimateResponse, CreateTrainingJobRequest, TrainingJobResponse,
};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::adapter_download_service::AdapterDownloadService;
use crate::services::audit_logger::AuditLogger;
use crate::services::model_service::ModelService;
use crate::services::plan_service::PlanService;
use crate::services::training_job_service::TrainingJobService;
use crate::temporal::TraceContext;

/// Training and model routes.
pub fn router() -> Router<AppState> {
    Router::new()
        // Training jobs
        .route(
            "/projects/{project_id}/training-jobs",
            post(create_training_job).get(list_training_jobs),
        )
        .route("/training-jobs/{id}", get(get_training_job))
        .route("/training-jobs/{id}/cancel", post(cancel_training_job))
        .route(
            "/training-jobs/{id}/approve-cost",
            post(approve_training_cost),
        )
        .route(
            "/projects/{project_id}/training-jobs/estimate",
            post(estimate_training_cost),
        )
        // Training status + metrics streams
        .route(
            "/training-jobs/{id}/status/stream",
            get(stream_training_status),
        )
        .route(
            "/training-jobs/{id}/metrics/stream",
            get(stream_training_metrics),
        )
        .route("/training-jobs/{id}/metrics", get(get_training_metrics))
        // Models
        .route("/projects/{project_id}/models", get(list_models))
        .route("/models/{id}", get(get_model))
        .route("/models/{id}/versions", get(list_model_versions))
        .route("/models/{id}/rollback", post(rollback_model))
        .route("/models/{id}/adapter/download", get(download_adapter))
}

/// POST /api/v1/projects/:project_id/training-jobs
///
/// Create a training job and auto-trigger the TrainWorkflow.
#[utoipa::path(
    post,
    path = "/api/v1/projects/{project_id}/training-jobs",
    tag = "Training",
    params(
        ("project_id" = Uuid, Path, description = "Project ID")
    ),
    request_body = CreateTrainingJobRequest,
    responses(
        (status = 201, description = "Training job created", body = TrainingJobResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn create_training_job(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    headers: HeaderMap,
    Path(project_id): Path<Uuid>,
    Json(body): Json<CreateTrainingJobRequest>,
) -> AppResult<(StatusCode, Json<TrainingJobResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let trace_ctx = TraceContext::from_headers(&headers);
    // Atomic limit check: INSERT ... WHERE count < max — no TOCTOU race.
    let limits = PlanService::get_limits(state.tenant_repo(), user.tenant_id).await?;
    let plan = PlanService::get_plan(state.tenant_repo(), user.tenant_id).await?;
    let base_model = body.base_model.clone();
    let result = TrainingJobService::create(
        state.training_job_repo(),
        state.dataset_repo(),
        state.tenant_repo(),
        state.billing_event_repo(),
        state.model_repo(),
        state.orchestrator(),
        state.secret_cipher(),
        user.tenant_id,
        project_id,
        body,
        Some(limits.max_models),
        None, // use default cost approval threshold
        state.config().teacher_gpu_spend_cap(&plan),
        state.config().on_policy_tokens_per_sec,
        trace_ctx,
    )
    .await?;

    let mut audit_meta = serde_json::json!({
        "base_model": base_model,
        "project_id": project_id.to_string(),
        "mode": result.mode.to_string(),
    });
    if let Some(teacher) = &result.teacher {
        audit_meta["teacher_host"] = serde_json::json!(teacher.host);
        audit_meta["teacher_model"] = serde_json::json!(teacher.model);
    }
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "create",
        "training_job",
        result.id.parse().ok(),
        audit_meta,
    )
    .await;

    Ok((StatusCode::CREATED, Json(result)))
}

/// POST /api/v1/projects/:project_id/training-jobs/estimate
///
/// Estimate training cost without creating a job.
#[utoipa::path(
    post,
    path = "/api/v1/projects/{project_id}/training-jobs/estimate",
    tag = "Training",
    params(
        ("project_id" = Uuid, Path, description = "Project ID")
    ),
    request_body = CreateTrainingJobRequest,
    responses(
        (status = 200, description = "Cost estimate", body = CostEstimateResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn estimate_training_cost(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(_project_id): Path<Uuid>,
    Json(body): Json<CreateTrainingJobRequest>,
) -> AppResult<Json<CostEstimateResponse>> {
    let result = TrainingJobService::estimate(
        state.dataset_repo(),
        state.tenant_repo(),
        user.tenant_id,
        &body,
    )
    .await?;
    Ok(Json(result))
}

/// GET /api/v1/projects/:project_id/training-jobs
#[utoipa::path(
    get,
    path = "/api/v1/projects/{project_id}/training-jobs",
    tag = "Training",
    params(
        ("project_id" = Uuid, Path, description = "Project ID"),
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
        ("limit" = Option<i64>, Query, description = "Pagination limit"),
    ),
    responses(
        (status = 200, description = "List of training jobs", body = inline(PaginatedResponse<TrainingJobResponse>)),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn list_training_jobs(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<TrainingJobResponse>>> {
    let result = TrainingJobService::list(
        state.training_job_repo(),
        user.tenant_id,
        project_id,
        params.offset(),
        params.limit(),
    )
    .await?;

    Ok(Json(result))
}

/// GET /api/v1/training-jobs/:id
#[utoipa::path(
    get,
    path = "/api/v1/training-jobs/{id}",
    tag = "Training",
    params(
        ("id" = Uuid, Path, description = "Training job ID")
    ),
    responses(
        (status = 200, description = "Training job details", body = TrainingJobResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn get_training_job(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<TrainingJobResponse>> {
    let job = TrainingJobService::get(state.training_job_repo(), user.tenant_id, id).await?;
    Ok(Json(job))
}

/// POST /api/v1/training-jobs/:id/approve-cost
///
/// Approve a training job that's waiting for cost approval.
#[utoipa::path(
    post,
    path = "/api/v1/training-jobs/{id}/approve-cost",
    tag = "Training",
    params(
        ("id" = Uuid, Path, description = "Training job ID")
    ),
    responses(
        (status = 200, description = "Cost approved, training started", body = TrainingJobResponse),
        (status = 400, description = "Job not in cost_approval status"),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn approve_training_cost(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Json<TrainingJobResponse>> {
    require_role(&user, TeamRole::Admin)?;
    let trace_ctx = TraceContext::from_headers(&headers);
    let job = TrainingJobService::approve_cost(
        state.training_job_repo(),
        state.dataset_repo(),
        state.orchestrator(),
        user.tenant_id,
        id,
        trace_ctx,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "approve_cost",
        "training_job",
        Some(id),
        serde_json::json!({"cost_estimate": job.cost_estimate}),
    )
    .await;

    Ok(Json(job))
}

/// POST /api/v1/training-jobs/:id/cancel
#[utoipa::path(
    post,
    path = "/api/v1/training-jobs/{id}/cancel",
    tag = "Training",
    params(
        ("id" = Uuid, Path, description = "Training job ID")
    ),
    responses(
        (status = 200, description = "Training job cancelled", body = TrainingJobResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn cancel_training_job(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<TrainingJobResponse>> {
    require_role(&user, TeamRole::Member)?;
    let job = TrainingJobService::cancel(
        state.training_job_repo(),
        state.tenant_repo(),
        state.orchestrator(),
        user.tenant_id,
        id,
    )
    .await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "cancel",
        "training_job",
        Some(id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(job))
}

/// GET /api/v1/training-jobs/:id/status/stream
///
/// SSE endpoint that pushes training job status changes. Polls DB server-side
/// every 3s and only emits when the status field changes, converting N client
/// polls into 1 server-side poll regardless of connected clients.
pub async fn stream_training_status(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    use platform_shared::enums::TrainingJobStatus;

    let initial = TrainingJobService::get(state.training_job_repo(), user.tenant_id, id).await?;
    let tenant_id = user.tenant_id;

    let stream = async_stream::stream! {
        let mut last_status = initial.status;

        if let Ok(json) = serde_json::to_string(&initial) {
            yield Ok(Event::default().data(json).event("status"));
        }

        let terminal = |s: TrainingJobStatus| matches!(
            s,
            TrainingJobStatus::Completed | TrainingJobStatus::Failed | TrainingJobStatus::Cancelled
        );

        if terminal(last_status) {
            return;
        }

        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;

            match TrainingJobService::get(state.training_job_repo(), tenant_id, id).await {
                Ok(job) => {
                    if job.status != last_status {
                        last_status = job.status;
                        if let Ok(json) = serde_json::to_string(&job) {
                            yield Ok(Event::default().data(json).event("status"));
                        }
                        if terminal(last_status) {
                            return;
                        }
                    } else {
                        yield Ok(Event::default().comment("heartbeat"));
                    }
                }
                Err(e) => {
                    // Kept alive rather than closed: a transient read failure must
                    // not look to the client like the job stopped changing.
                    tracing::warn!(training_job_id = %id, error = ?e, "Status poll failed");
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

/// GET /api/v1/training-jobs/:id/metrics/stream
///
/// SSE endpoint that streams real-time training metrics from Redis.
#[utoipa::path(
    get,
    path = "/api/v1/training-jobs/{id}/metrics/stream",
    tag = "Training",
    params(
        ("id" = Uuid, Path, description = "Training job ID")
    ),
    responses(
        (status = 200, description = "SSE stream", content_type = "text/event-stream"),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn stream_training_metrics(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    // Tenant ownership check — prevents cross-tenant IDOR on metrics stream
    TrainingJobService::get(state.training_job_repo(), user.tenant_id, id).await?;

    let mut redis = state.redis();
    let stream_key = format!(
        "{}{}",
        platform_shared::constants::REDIS_TRAINING_METRICS_STREAM,
        id
    );

    let stream = async_stream::stream! {
        let mut last_id = "0-0".to_string();

        loop {
            // XREAD with 3s block timeout
            let result: Result<redis::Value, redis::RedisError> = redis::cmd("XREAD")
                .arg("COUNT")
                .arg(10)
                .arg("BLOCK")
                .arg(3000)
                .arg("STREAMS")
                .arg(&stream_key)
                .arg(&last_id)
                .query_async(&mut redis)
                .await;

            match result {
                Ok(redis::Value::Array(streams)) => {
                    for stream_data in streams {
                        if let redis::Value::Array(stream_entries) = stream_data {
                            // stream_entries: [stream_name, [[id, [field, value, ...]], ...]]
                            if let Some(redis::Value::Array(entries)) = stream_entries.get(1) {
                                for entry in entries {
                                    if let redis::Value::Array(entry_parts) = entry {
                                        // entry_parts: [id, [field, value, ...]]
                                        if let Some(redis::Value::BulkString(entry_id)) = entry_parts.first() {
                                            last_id = String::from_utf8_lossy(entry_id).to_string();
                                        }

                                        if let Some(redis::Value::Array(fields)) = entry_parts.get(1) {
                                            let data = redis_fields_to_json(fields);
                                            let event = Event::default()
                                                .data(data.to_string())
                                                .event("metrics");
                                            yield Ok(event);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(redis::Value::Nil) => {
                    // No new data (block timeout), send heartbeat
                    yield Ok(Event::default().comment("heartbeat"));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Redis XREAD error in metrics stream");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    yield Ok(Event::default().comment("reconnecting"));
                }
                _ => {
                    // Nil or other response, send heartbeat
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

/// GET /api/v1/training-jobs/:id/metrics
///
/// Get the latest training metrics snapshot from the job record.
#[utoipa::path(
    get,
    path = "/api/v1/training-jobs/{id}/metrics",
    tag = "Training",
    params(
        ("id" = Uuid, Path, description = "Training job ID")
    ),
    responses(
        (status = 200, description = "Training metrics snapshot", body = serde_json::Value),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn get_training_metrics(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let job = TrainingJobService::get(state.training_job_repo(), user.tenant_id, id).await?;
    Ok(Json(serde_json::to_value(&job.metrics).unwrap_or_default()))
}

/// GET /api/v1/projects/:project_id/models
#[utoipa::path(
    get,
    path = "/api/v1/projects/{project_id}/models",
    tag = "Training",
    params(
        ("project_id" = Uuid, Path, description = "Project ID"),
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
        ("limit" = Option<i64>, Query, description = "Pagination limit"),
    ),
    responses(
        (status = 200, description = "List of models", body = inline(PaginatedResponse<ModelResponse>)),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn list_models(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<ModelResponse>>> {
    let result = ModelService::list(
        state.model_repo(),
        user.tenant_id,
        project_id,
        params.offset(),
        params.limit(),
    )
    .await?;

    Ok(Json(result))
}

/// GET /api/v1/models/:id
#[utoipa::path(
    get,
    path = "/api/v1/models/{id}",
    tag = "Training",
    params(
        ("id" = Uuid, Path, description = "Model ID")
    ),
    responses(
        (status = 200, description = "Model details", body = ModelResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn get_model(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ModelResponse>> {
    let model = ModelService::get(state.model_repo(), user.tenant_id, id).await?;
    Ok(Json(model))
}

/// GET /api/v1/models/:id/adapter/download
#[utoipa::path(
    get,
    path = "/api/v1/models/{id}/adapter/download",
    tag = "Models",
    params(("id" = Uuid, Path, description = "Model ID")),
    responses(
        (status = 200, description = "Adapter archive", content_type = "application/zip"),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn download_adapter(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<impl axum::response::IntoResponse> {
    let archive = AdapterDownloadService::build_archive(
        state.model_repo(),
        state.storage(),
        user.tenant_id,
        id,
        state.config().adapter_download_max_bytes,
    )
    .await?;

    let headers = [
        (
            axum::http::header::CONTENT_TYPE,
            "application/zip".to_string(),
        ),
        (
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", archive.filename),
        ),
    ];

    Ok((headers, archive.bytes))
}

/// GET /api/v1/models/:id/versions
///
/// List all versions of a model (same base_model within a project).
pub async fn list_model_versions(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<ModelResponse>>> {
    let versions = ModelService::list_versions(state.model_repo(), user.tenant_id, id).await?;
    Ok(Json(versions))
}

/// POST /api/v1/models/:id/rollback
///
/// Deploy a previous version and undeploy the current one.
pub async fn rollback_model(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RollbackModelRequest>,
) -> AppResult<Json<ModelResponse>> {
    require_role(&user, TeamRole::Admin)?;

    let target_id: Uuid =
        body.target_version_id
            .parse()
            .map_err(|_| crate::error::AppError::BadRequest {
                message: "Invalid target_version_id".to_string(),
            })?;

    let result = ModelService::rollback(&state, user.tenant_id, id, target_id).await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "rollback",
        "model",
        Some(id),
        serde_json::json!({
            "target_version_id": body.target_version_id,
            "target_version": result.version,
        }),
    )
    .await;

    Ok(Json(result))
}

/// Convert Redis stream field/value pairs to a JSON object.
fn redis_fields_to_json(fields: &[redis::Value]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let mut iter = fields.iter();

    while let (Some(key), Some(value)) = (iter.next(), iter.next()) {
        if let (redis::Value::BulkString(k), redis::Value::BulkString(v)) = (key, value) {
            let key_str = String::from_utf8_lossy(k).to_string();
            let val_str = String::from_utf8_lossy(v).to_string();

            // Try to parse as number, fall back to string
            if let Ok(n) = val_str.parse::<f64>() {
                map.insert(key_str, serde_json::Value::from(n));
            } else {
                map.insert(key_str, serde_json::Value::from(val_str));
            }
        }
    }

    serde_json::Value::Object(map)
}
