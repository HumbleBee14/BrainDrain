use axum::extract::multipart::Multipart;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use platform_shared::constants::MAX_DATASET_IMPORT_BYTES;
use platform_shared::enums::TeamRole;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::dataset::{DatasetImportResponse, DatasetResponse};
use crate::error::{AppError, AppResult};
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::dataset_service::DatasetService;

/// Dataset routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/datasets", get(list_datasets))
        // Raise the body limit for JSONL import ONLY (scoped to this POST),
        // matching the document-upload pattern; the handler enforces the real
        // cap while buffering. JSON routes keep the small default limit.
        .route(
            "/projects/{project_id}/datasets/import",
            post(import_dataset).layer(DefaultBodyLimit::max(MAX_DATASET_IMPORT_BYTES as usize)),
        )
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
        params.offset(),
        params.limit(),
    )
    .await?;

    Ok(Json(result))
}

/// Import an OpenAI-format chat JSONL dataset into a project.
///
/// Multipart form: a `file` field with the JSONL, and an optional `name` text
/// field. Malformed rows are reported per row rather than failing the whole
/// file; the created dataset enters the standard `review_pending` review flow.
#[utoipa::path(
    post,
    path = "/api/v1/projects/{project_id}/datasets/import",
    tag = "Datasets",
    params(("project_id" = Uuid, Path, description = "Project ID")),
    request_body(content_type = "multipart/form-data", content = String, description = "JSONL file upload"),
    responses(
        (status = 201, description = "Dataset imported", body = DatasetImportResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn import_dataset(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<DatasetImportResponse>)> {
    require_role(&user, TeamRole::Member)?;

    let mut file_bytes: Option<Vec<u8>> = None;
    let mut name: Option<String> = None;
    let mut file_name: Option<String> = None;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest {
            message: format!("Invalid multipart data: {e}"),
        })?
    {
        match field.name() {
            Some("name") => {
                name = Some(field.text().await.map_err(|e| AppError::BadRequest {
                    message: format!("Invalid 'name' field: {e}"),
                })?);
            }
            _ => {
                // Any other field is treated as the file payload. Buffer it,
                // enforcing the size cap chunk-by-chunk before it grows unbounded.
                file_name = field.file_name().map(|s| s.to_string());
                let mut buf: Vec<u8> = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(|e| AppError::BadRequest {
                    message: format!("Failed to read upload data: {e}"),
                })? {
                    if buf.len() as u64 + chunk.len() as u64 > MAX_DATASET_IMPORT_BYTES {
                        return Err(AppError::BadRequest {
                            message: format!(
                                "File exceeds maximum size of {} MB",
                                MAX_DATASET_IMPORT_BYTES / 1024 / 1024
                            ),
                        });
                    }
                    buf.extend_from_slice(&chunk);
                }
                file_bytes = Some(buf);
            }
        }
    }

    let file_bytes = file_bytes.ok_or(AppError::BadRequest {
        message: "No file provided".to_string(),
    })?;

    let content = String::from_utf8(file_bytes).map_err(|_| AppError::BadRequest {
        message: "Uploaded file is not valid UTF-8 text".to_string(),
    })?;

    // Dataset name: explicit form field, else the filename, else a default.
    let dataset_name = name
        .filter(|s| !s.trim().is_empty())
        .or(file_name)
        .unwrap_or_else(|| "Imported dataset".to_string());

    let result = DatasetService::import_openai_jsonl(
        state.dataset_repo(),
        state.project_repo(),
        state.storage(),
        user.tenant_id,
        project_id,
        &dataset_name,
        &content,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "import",
        "dataset",
        Some(Uuid::parse_str(&result.dataset.id).unwrap_or(project_id)),
        serde_json::json!({
            "imported_rows": result.imported_rows,
            "rejected_rows": result.rejected_rows,
        }),
    )
    .await;

    Ok((StatusCode::CREATED, Json(result)))
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
    let max_rows = params.max_rows.clamp(1, 200);
    let rows = DatasetService::preview(
        state.dataset_repo(),
        state.storage(),
        user.tenant_id,
        id,
        max_rows,
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
    require_role(&user, TeamRole::Admin)?;
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
    require_role(&user, TeamRole::Admin)?;
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
