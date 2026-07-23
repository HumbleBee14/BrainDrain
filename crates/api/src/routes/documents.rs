use axum::extract::multipart::Multipart;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::BytesMut;
use uuid::Uuid;

use platform_shared::constants::MAX_UPLOAD_SIZE_BYTES;
use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::document::{DocumentResponse, UploadResponse};
use crate::error::{AppError, AppResult};
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::document_service::DocumentService;

/// Document routes nested under projects.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/documents", post(upload_document))
        .route("/projects/{project_id}/documents", get(list_documents))
        .route("/documents/{id}", get(get_document))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{project_id}/documents",
    tag = "Documents",
    params(("project_id" = Uuid, Path, description = "Project ID")),
    request_body(content_type = "multipart/form-data", content = String, description = "File upload"),
    responses(
        (status = 201, description = "Documents uploaded", body = Vec<UploadResponse>),
        (status = 400, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn upload_document(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<Vec<UploadResponse>>)> {
    require_role(&user, TeamRole::Member)?;
    let mut uploads = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest {
            message: format!("Invalid multipart data: {e}"),
        })?
    {
        let filename = field
            .file_name()
            .map(|s| s.to_string())
            .ok_or(AppError::BadRequest {
                message: "File field must have a filename".to_string(),
            })?;

        let content_type = field
            .content_type()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        // Read file data into memory (with size limit)
        let mut data = BytesMut::new();
        let mut field = field;

        // Read chunks manually to enforce size limit
        while let Some(chunk) = field.chunk().await.map_err(|e| AppError::BadRequest {
            message: format!("Failed to read upload data: {e}"),
        })? {
            data.extend_from_slice(&chunk);
            if data.len() as u64 > MAX_UPLOAD_SIZE_BYTES {
                return Err(AppError::BadRequest {
                    message: format!(
                        "File exceeds maximum size of {} MB",
                        MAX_UPLOAD_SIZE_BYTES / 1024 / 1024
                    ),
                });
            }
        }

        let result = DocumentService::upload(
            state.document_repo(),
            state.tenant_repo(),
            state.storage(),
            user.tenant_id,
            project_id,
            &filename,
            &content_type,
            data.freeze(),
        )
        .await?;

        uploads.push(result);
    }

    if uploads.is_empty() {
        return Err(AppError::BadRequest {
            message: "No files provided".to_string(),
        });
    }

    for upload in &uploads {
        AuditLogger::log(
            state.audit_log_repo(),
            &user,
            "create",
            "document",
            Some(upload.id),
            serde_json::json!({"filename": upload.filename, "file_size": upload.file_size}),
        )
        .await;
    }

    Ok((StatusCode::CREATED, Json(uploads)))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{project_id}/documents",
    tag = "Documents",
    params(
        ("project_id" = Uuid, Path, description = "Project ID"),
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
        ("limit" = Option<i64>, Query, description = "Pagination limit"),
    ),
    responses(
        (status = 200, description = "List of documents", body = inline(PaginatedResponse<DocumentResponse>)),
    ),
    security(("jwt" = []))
)]
pub async fn list_documents(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Query(params): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<DocumentResponse>>> {
    let result = DocumentService::list(
        state.document_repo(),
        user.tenant_id,
        project_id,
        params.offset(),
        params.limit(),
    )
    .await?;
    Ok(Json(result))
}

#[utoipa::path(
    get,
    path = "/api/v1/documents/{id}",
    tag = "Documents",
    params(("id" = Uuid, Path, description = "Document ID")),
    responses(
        (status = 200, description = "Document details", body = DocumentResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn get_document(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<DocumentResponse>> {
    let doc = DocumentService::get(state.document_repo(), user.tenant_id, id).await?;
    Ok(Json(doc))
}
