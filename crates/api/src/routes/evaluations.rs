use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::evaluation::{CreateEvaluationRequest, EvaluationResponse};
use crate::error::AppResult;
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
async fn create_evaluation(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
    Json(body): Json<CreateEvaluationRequest>,
) -> AppResult<(StatusCode, Json<EvaluationResponse>)> {
    let result =
        EvaluationService::create(state.db(), state.temporal(), user.tenant_id, model_id, body)
            .await?;

    Ok((StatusCode::CREATED, Json(result)))
}

/// GET /api/v1/models/:model_id/evaluations
async fn list_evaluations(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<EvaluationResponse>>> {
    let result = EvaluationService::list(
        state.db(),
        user.tenant_id,
        model_id,
        params.offset,
        params.limit,
    )
    .await?;

    Ok(Json(result))
}

/// GET /api/v1/evaluations/:id
async fn get_evaluation(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<EvaluationResponse>> {
    let eval = EvaluationService::get(state.db(), user.tenant_id, id).await?;
    Ok(Json(eval))
}
