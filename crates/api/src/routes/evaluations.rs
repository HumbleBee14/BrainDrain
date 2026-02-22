use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::evaluation::{CreateEvaluationRequest, EvaluationResponse};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::evaluation_service::EvaluationService;

/// Evaluation routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/models/{model_id}/evaluations",
            post(create_evaluation).get(list_evaluations),
        )
        .route("/evaluations/{id}", get(get_evaluation))
}

/// POST /api/v1/models/:model_id/evaluations
#[utoipa::path(
    post,
    path = "/api/v1/models/{model_id}/evaluations",
    tag = "Evaluations",
    params(
        ("model_id" = Uuid, Path, description = "Model ID")
    ),
    request_body = CreateEvaluationRequest,
    responses(
        (status = 201, description = "Evaluation created", body = EvaluationResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn create_evaluation(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
    Json(body): Json<CreateEvaluationRequest>,
) -> AppResult<(StatusCode, Json<EvaluationResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let result = EvaluationService::create(
        state.evaluation_repo(),
        state.model_repo(),
        state.training_job_repo(),
        state.dataset_repo(),
        state.orchestrator(),
        user.tenant_id,
        model_id,
        body,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "create",
        "evaluation",
        result.id.parse().ok(),
        serde_json::json!({"model_id": model_id.to_string()}),
    )
    .await;

    Ok((StatusCode::CREATED, Json(result)))
}

/// GET /api/v1/models/:model_id/evaluations
#[utoipa::path(
    get,
    path = "/api/v1/models/{model_id}/evaluations",
    tag = "Evaluations",
    params(
        ("model_id" = Uuid, Path, description = "Model ID"),
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
        ("limit" = Option<i64>, Query, description = "Pagination limit"),
    ),
    responses(
        (status = 200, description = "List of evaluations", body = inline(PaginatedResponse<EvaluationResponse>)),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn list_evaluations(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<EvaluationResponse>>> {
    let result = EvaluationService::list(
        state.evaluation_repo(),
        user.tenant_id,
        model_id,
        params.offset,
        params.limit,
    )
    .await?;

    Ok(Json(result))
}

/// GET /api/v1/evaluations/:id
#[utoipa::path(
    get,
    path = "/api/v1/evaluations/{id}",
    tag = "Evaluations",
    params(
        ("id" = Uuid, Path, description = "Evaluation ID")
    ),
    responses(
        (status = 200, description = "Evaluation details", body = EvaluationResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn get_evaluation(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<EvaluationResponse>> {
    let eval = EvaluationService::get(state.evaluation_repo(), user.tenant_id, id).await?;
    Ok(Json(eval))
}
