use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use platform_shared::enums::{FeedbackRating, TeamRole};

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::auth_api_key::ApiKeyAuth;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::feedback::{
    ApiFeedbackRequest, InferenceSampleResponse, PromoteSamplesRequest, PromoteSamplesResponse,
    SetCaptureRequest, SubmitFeedbackRequest,
};
use crate::error::{AppError, AppResult};
use crate::rbac::require_role;
use crate::services::inference_sample_service::InferenceSampleService;

/// Dashboard feedback routes (JWT auth, mounted under /api/v1).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/models/{model_id}/samples", get(list_samples))
        .route("/models/{model_id}/samples/promote", post(promote_samples))
        .route("/models/{model_id}/capture", put(set_capture))
        .route("/samples/{id}/feedback", post(submit_feedback))
}

/// End-user feedback route (API key auth, mounted at /v1 alongside inference).
pub fn api_router() -> Router<AppState> {
    Router::new().route("/v1/feedback", post(submit_api_feedback))
}

#[derive(Debug, Deserialize)]
pub struct SampleListParams {
    #[serde(flatten)]
    pagination: PaginationParams,
    /// "positive" | "negative" | "unrated" | absent (all).
    rating: Option<String>,
}

fn parse_rating_filter(raw: Option<&str>) -> AppResult<(Option<FeedbackRating>, bool)> {
    match raw {
        None => Ok((None, false)),
        Some("unrated") => Ok((None, true)),
        Some(value) => {
            let rating = value.parse().map_err(|_| AppError::BadRequest {
                message: format!(
                    "Invalid rating filter '{value}' (expected positive, negative, or unrated)"
                ),
            })?;
            Ok((Some(rating), false))
        }
    }
}

/// GET /api/v1/models/:model_id/samples
#[utoipa::path(
    get,
    path = "/api/v1/models/{model_id}/samples",
    tag = "Feedback",
    params(
        ("model_id" = Uuid, Path, description = "Model ID"),
        ("rating" = Option<String>, Query, description = "Filter: positive, negative, or unrated"),
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
        ("limit" = Option<i64>, Query, description = "Pagination limit"),
    ),
    responses(
        (status = 200, description = "Captured inference samples", body = inline(PaginatedResponse<InferenceSampleResponse>)),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn list_samples(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
    Query(params): Query<SampleListParams>,
) -> AppResult<Json<PaginatedResponse<InferenceSampleResponse>>> {
    let (rating, unrated_only) = parse_rating_filter(params.rating.as_deref())?;
    let result = InferenceSampleService::list(
        state.inference_sample_repo(),
        user.tenant_id,
        model_id,
        rating,
        unrated_only,
        params.pagination.offset(),
        params.pagination.limit(),
    )
    .await?;
    Ok(Json(result))
}

/// POST /api/v1/models/:model_id/samples/promote
#[utoipa::path(
    post,
    path = "/api/v1/models/{model_id}/samples/promote",
    tag = "Feedback",
    params(("model_id" = Uuid, Path, description = "Model ID")),
    request_body = PromoteSamplesRequest,
    responses(
        (status = 201, description = "Training dataset created from samples", body = PromoteSamplesResponse),
        (status = 400, description = "Invalid selection (e.g. negative sample without correction)"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Model or sample not found"),
    ),
    security(("jwt" = []))
)]
pub async fn promote_samples(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
    Json(body): Json<PromoteSamplesRequest>,
) -> AppResult<(StatusCode, Json<PromoteSamplesResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let result = InferenceSampleService::promote(
        state.inference_sample_repo(),
        state.model_repo(),
        state.dataset_repo(),
        state.storage(),
        user.tenant_id,
        model_id,
        body,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

/// PUT /api/v1/models/:model_id/capture
#[utoipa::path(
    put,
    path = "/api/v1/models/{model_id}/capture",
    tag = "Feedback",
    params(("model_id" = Uuid, Path, description = "Model ID")),
    request_body = SetCaptureRequest,
    responses(
        (status = 204, description = "Capture setting updated"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Model not found"),
    ),
    security(("jwt" = []))
)]
pub async fn set_capture(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
    Json(body): Json<SetCaptureRequest>,
) -> AppResult<StatusCode> {
    require_role(&user, TeamRole::Member)?;
    InferenceSampleService::set_capture(state.model_repo(), user.tenant_id, model_id, body.enabled)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/samples/:id/feedback
#[utoipa::path(
    post,
    path = "/api/v1/samples/{id}/feedback",
    tag = "Feedback",
    params(("id" = Uuid, Path, description = "Sample ID")),
    request_body = SubmitFeedbackRequest,
    responses(
        (status = 204, description = "Feedback recorded"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Sample not found"),
    ),
    security(("jwt" = []))
)]
pub async fn submit_feedback(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<SubmitFeedbackRequest>,
) -> AppResult<StatusCode> {
    require_role(&user, TeamRole::Member)?;
    InferenceSampleService::submit_feedback(
        state.inference_sample_repo(),
        user.tenant_id,
        id,
        body.rating,
        body.comment,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/feedback — end-user feedback via the OpenAI-compatible API.
///
/// The caller references the `x-sample-id` header returned by a captured
/// chat completion. The API key's model scope is enforced.
#[utoipa::path(
    post,
    path = "/v1/feedback",
    tag = "Feedback",
    request_body = ApiFeedbackRequest,
    responses(
        (status = 204, description = "Feedback recorded"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Sample not found"),
    ),
    security(("api_key" = []))
)]
pub async fn submit_api_feedback(
    State(state): State<AppState>,
    api_key: ApiKeyAuth,
    Json(body): Json<ApiFeedbackRequest>,
) -> AppResult<StatusCode> {
    let sample_id: Uuid = body.sample_id.parse().map_err(|_| AppError::BadRequest {
        message: "sample_id must be a UUID".to_string(),
    })?;
    InferenceSampleService::submit_api_feedback(
        state.inference_sample_repo(),
        api_key.tenant_id,
        api_key.model_id,
        sample_id,
        body.rating,
        body.comment,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_filter_parses_all_variants() {
        assert_eq!(parse_rating_filter(None).unwrap(), (None, false));
        assert_eq!(parse_rating_filter(Some("unrated")).unwrap(), (None, true));
        assert_eq!(
            parse_rating_filter(Some("positive")).unwrap(),
            (Some(FeedbackRating::Positive), false)
        );
        assert_eq!(
            parse_rating_filter(Some("negative")).unwrap(),
            (Some(FeedbackRating::Negative), false)
        );
        assert!(parse_rating_filter(Some("bogus")).is_err());
    }
}
