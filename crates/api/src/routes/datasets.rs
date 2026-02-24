use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use platform_shared::enums::TeamRole;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::dataset::DatasetResponse;
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::dataset_service::DatasetService;

/// Dataset routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/datasets", get(list_datasets))
        .route("/datasets/{id}", get(get_dataset))
        .route("/datasets/{id}/preview", get(preview_dataset))
        .route("/datasets/{id}/approve", post(approve_dataset))
        .route("/datasets/{id}/reject", post(reject_dataset))
        .route("/documents/{id}/parsed", get(get_parsed_content))
}

/// List datasets for a project.
#[utoipa::path(
    get,
    path = "/api/v1/projects/{project_id}/datasets",
    tag = "Datasets",
    params(
        ("project_id" = Uuid, Path, description = "Project ID"),
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
        ("limit" = Option<i64>, Query, description = "Pagination limit"),
    ),
    responses(
        (status = 200, description = "List of datasets", body = inline(PaginatedResponse<DatasetResponse>)),
    ),
    security(("jwt" = []))
)]
pub async fn list_datasets(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<DatasetResponse>>> {
    let result = DatasetService::list(
        state.dataset_repo(),
        user.tenant_id,
        project_id,
        params.offset,
        params.limit,
    )
    .await?;

    Ok(Json(result))
}

/// Get a single dataset by ID.
#[utoipa::path(
    get,
    path = "/api/v1/datasets/{id}",
    tag = "Datasets",
    params(("id" = Uuid, Path, description = "Dataset ID")),
    responses(
        (status = 200, description = "Dataset details", body = DatasetResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn get_dataset(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<DatasetResponse>> {
    let dataset = DatasetService::get(state.dataset_repo(), user.tenant_id, id).await?;
    Ok(Json(dataset))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PreviewParams {
    #[serde(default = "default_max_rows")]
    max_rows: usize,
}

fn default_max_rows() -> usize {
    20
}

/// Preview dataset rows.
#[utoipa::path(
    get,
    path = "/api/v1/datasets/{id}/preview",
    tag = "Datasets",
    params(
        ("id" = Uuid, Path, description = "Dataset ID"),
        PreviewParams,
    ),
    responses(
        (status = 200, description = "Dataset preview rows", body = Vec<serde_json::Value>),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn preview_dataset(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Query(params): Query<PreviewParams>,
) -> AppResult<Json<Vec<serde_json::Value>>> {
    let rows = DatasetService::preview(
        state.dataset_repo(),
        state.storage(),
        user.tenant_id,
        id,
        params.max_rows,
    )
    .await?;

    Ok(Json(rows))
}

/// Approve a dataset for training.
#[utoipa::path(
    post,
    path = "/api/v1/datasets/{id}/approve",
    tag = "Datasets",
    params(("id" = Uuid, Path, description = "Dataset ID")),
    responses(
        (status = 200, description = "Dataset approved", body = DatasetResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn approve_dataset(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<DatasetResponse>> {
    require_role(&user, TeamRole::Member)?;
    let dataset = DatasetService::approve(state.dataset_repo(), user.tenant_id, id).await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "approve",
        "dataset",
        Some(id),
        serde_json::json!({}),
    )
    .await;

    Ok(Json(dataset))
}

/// Reject a dataset (archives it).
#[utoipa::path(
    post,
    path = "/api/v1/datasets/{id}/reject",
    tag = "Datasets",
    params(("id" = Uuid, Path, description = "Dataset ID")),
    responses(
        (status = 200, description = "Dataset rejected", body = DatasetResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn reject_dataset(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<DatasetResponse>> {
    require_role(&user, TeamRole::Member)?;
    let dataset = DatasetService::reject(state.dataset_repo(), user.tenant_id, id).await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "reject",
        "dataset",
        Some(id),
        serde_json::json!({}),
    )
    .await;

    Ok(Json(dataset))
}

/// Get presigned URL for parsed document content.
#[utoipa::path(
    get,
    path = "/api/v1/documents/{id}/parsed",
    tag = "Datasets",
    params(("id" = Uuid, Path, description = "Document ID")),
    responses(
        (status = 200, description = "Presigned URL for parsed content", body = ParsedContentResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn get_parsed_content(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ParsedContentResponse>> {
    let doc = crate::services::document_service::DocumentService::get(
        state.document_repo(),
        user.tenant_id,
        id,
    )
    .await?;

    let url =
        DatasetService::get_parsed_url(state.storage(), user.tenant_id, doc.project_id, id).await?;

    Ok(Json(ParsedContentResponse { url }))
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct ParsedContentResponse {
    url: String,
}
