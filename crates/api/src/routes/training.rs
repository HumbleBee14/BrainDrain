use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::model::ModelResponse;
use crate::dto::training_job::{
    CostEstimateResponse, CreateTrainingJobRequest, TrainingJobResponse,
};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::model_service::ModelService;
use crate::services::plan_service::PlanService;
use crate::services::training_job_service::TrainingJobService;

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
        // Training metrics
        .route(
            "/training-jobs/{id}/metrics/stream",
            get(stream_training_metrics),
        )
        .route("/training-jobs/{id}/metrics", get(get_training_metrics))
        // Models
        .route("/projects/{project_id}/models", get(list_models))
        .route("/models/{id}", get(get_model))
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
    Path(project_id): Path<Uuid>,
    Json(body): Json<CreateTrainingJobRequest>,
) -> AppResult<(StatusCode, Json<TrainingJobResponse>)> {
    require_role(&user, TeamRole::Member)?;
    // Atomic limit check: INSERT ... WHERE count < max — no TOCTOU race.
    let limits = PlanService::get_limits(state.tenant_repo(), user.tenant_id).await?;
    let base_model = body.base_model.clone();
    let result = TrainingJobService::create(
        state.training_job_repo(),
        state.dataset_repo(),
        state.orchestrator(),
        user.tenant_id,
        project_id,
        body,
        Some(limits.max_models),
        None, // use default cost approval threshold
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "create",
        "training_job",
        result.id.parse().ok(),
        serde_json::json!({"base_model": base_model, "project_id": project_id.to_string()}),
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
    let result = TrainingJobService::estimate(state.dataset_repo(), user.tenant_id, &body).await?;
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
        params.offset,
        params.limit,
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
    Path(id): Path<Uuid>,
) -> AppResult<Json<TrainingJobResponse>> {
    require_role(&user, TeamRole::Admin)?;
    let job = TrainingJobService::approve_cost(
        state.training_job_repo(),
        state.dataset_repo(),
        state.orchestrator(),
        user.tenant_id,
        id,
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
    let job = TrainingJobService::cancel(state.training_job_repo(), user.tenant_id, id).await?;
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
    _user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
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

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
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
        params.offset,
        params.limit,
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
