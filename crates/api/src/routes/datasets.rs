use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::dataset::DatasetResponse;
use crate::error::AppResult;
use crate::services::dataset_service::DatasetService;

/// Dataset routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/datasets", get(list_datasets))
        .route("/datasets/{id}", get(get_dataset))
        .route("/datasets/{id}/preview", get(preview_dataset))
        .route("/documents/{id}/parsed", get(get_parsed_content))
}

/// GET /api/v1/projects/:project_id/datasets
async fn list_datasets(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<DatasetResponse>>> {
    let result = DatasetService::list(
        state.db(),
        user.tenant_id,
        project_id,
        params.offset,
        params.limit,
    )
    .await?;

    Ok(Json(result))
}

/// GET /api/v1/datasets/:id
async fn get_dataset(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<DatasetResponse>> {
    let dataset = DatasetService::get(state.db(), user.tenant_id, id).await?;
    Ok(Json(dataset))
}

#[derive(Debug, Deserialize)]
struct PreviewParams {
    #[serde(default = "default_max_rows")]
    max_rows: usize,
}

fn default_max_rows() -> usize {
    20
}

/// GET /api/v1/datasets/:id/preview
async fn preview_dataset(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Query(params): Query<PreviewParams>,
) -> AppResult<Json<Vec<serde_json::Value>>> {
    let rows = DatasetService::preview(
        state.db(),
        state.storage(),
        user.tenant_id,
        id,
        params.max_rows,
    )
    .await?;

    Ok(Json(rows))
}

/// GET /api/v1/documents/:id/parsed
///
/// Returns a presigned URL for the parsed content JSON.
async fn get_parsed_content(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ParsedContentResponse>> {
    let doc = crate::services::document_service::DocumentService::get(
        state.db(),
        user.tenant_id,
        id,
    )
    .await?;

    let url = DatasetService::get_parsed_url(
        state.storage(),
        user.tenant_id,
        doc.project_id,
        id,
    )
    .await?;

    Ok(Json(ParsedContentResponse { url }))
}

#[derive(Debug, serde::Serialize)]
struct ParsedContentResponse {
    url: String,
}
