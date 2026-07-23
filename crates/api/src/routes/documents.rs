use axum::extract::multipart::Multipart;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use platform_shared::constants::MAX_UPLOAD_SIZE_BYTES;
use platform_shared::enums::TeamRole;
use platform_storage::StorageError;

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
        // Raise the request-body limit for uploads ONLY. Axum's default 2 MB limit
        // would otherwise abort any file over 2 MB inside the Multipart extractor —
        // well below the advertised MAX_UPLOAD_SIZE_BYTES — surfacing as a confusing
        // "invalid multipart data" error. The handler still enforces the real cap
        // while streaming, before buffering. The layer is scoped to this POST method
        // router, so JSON routes keep the small default limit.
        .route(
            "/projects/{project_id}/documents",
            post(upload_document).layer(DefaultBodyLimit::max(MAX_UPLOAD_SIZE_BYTES as usize)),
        )
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

        let mut field = field;

        // Stream the field straight to storage over a bounded channel: the
        // producer reads chunks and enforces the size cap; the consumer
        // (the service) writes them to object storage. Neither side buffers
        // the whole file. The two run concurrently under one task via join!.
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, StorageError>>(4);

        let producer = async move {
            let mut total: u64 = 0;
            loop {
                match field.chunk().await {
                    Ok(Some(chunk)) => {
                        total += chunk.len() as u64;
                        if total > MAX_UPLOAD_SIZE_BYTES {
                            // Abort the storage write, then fail the request.
                            let _ = tx
                                .send(Err(StorageError::UploadFailed(
                                    "upload exceeds size cap".to_string(),
                                )))
                                .await;
                            return Err(AppError::BadRequest {
                                message: format!(
                                    "File exceeds maximum size of {} MB",
                                    MAX_UPLOAD_SIZE_BYTES / 1024 / 1024
                                ),
                            });
                        }
                        if tx.send(Ok(chunk)).await.is_err() {
                            // Consumer stopped early (storage error); stop reading.
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx
                            .send(Err(StorageError::UploadFailed(format!("read error: {e}"))))
                            .await;
                        return Err(AppError::BadRequest {
                            message: format!("Failed to read upload data: {e}"),
                        });
                    }
                }
            }
            Ok::<(), AppError>(())
        };

        let consumer = DocumentService::upload_streaming(
            state.document_repo(),
            state.tenant_repo(),
            state.storage(),
            user.tenant_id,
            project_id,
            &filename,
            &content_type,
            ReceiverStream::new(rx),
        );

        let (producer_result, consumer_result) = tokio::join!(producer, consumer);
        // A producer failure (size cap / read error) takes precedence — the
        // consumer's error in that case is just the abort it was told to do.
        producer_result?;
        uploads.push(consumer_result?);
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
