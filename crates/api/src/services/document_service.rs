use bytes::Bytes;
use sqlx::PgPool;
use uuid::Uuid;

use platform_shared::constants::SUPPORTED_EXTENSIONS;
use platform_shared::s3_paths;
use platform_storage::ObjectStorage;

use crate::dto::common::PaginatedResponse;
use crate::dto::document::{DocumentResponse, UploadResponse};
use crate::error::{AppError, AppResult};
use crate::repositories::document_repo::DocumentRepo;

/// Business logic for document operations.
///
/// Handles upload validation, S3 storage, DB record creation, and event publishing.
pub struct DocumentService;

impl DocumentService {
    /// Upload a document: validate → store in S3 → create DB record.
    pub async fn upload(
        db: &PgPool,
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
        project_id: Uuid,
        filename: &str,
        content_type: &str,
        data: Bytes,
    ) -> AppResult<UploadResponse> {
        // Validate file extension
        let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();

        if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
            return Err(AppError::BadRequest {
                message: format!(
                    "Unsupported file type '.{ext}'. Supported: {}",
                    SUPPORTED_EXTENSIONS.join(", ")
                ),
            });
        }

        let file_size = data.len() as i64;
        if file_size == 0 {
            return Err(AppError::BadRequest {
                message: "File is empty".to_string(),
            });
        }

        // Generate file ID and S3 path
        let file_id = Uuid::now_v7();
        let storage_path = s3_paths::upload_path(tenant_id, project_id, file_id, &ext);

        // Upload to S3
        storage
            .put(&storage_path, data, content_type)
            .await
            .map_err(AppError::Storage)?;

        // Create DB record
        let doc = DocumentRepo::create(
            db,
            tenant_id,
            project_id,
            filename,
            file_size,
            content_type,
            &storage_path,
        )
        .await?;

        tracing::info!(
            document_id = %doc.id,
            project_id = %project_id,
            filename = filename,
            file_size = file_size,
            "Document uploaded"
        );

        Ok(UploadResponse {
            id: doc.id,
            filename: doc.filename,
            file_size: doc.file_size,
            status: doc.status,
        })
    }

    /// List documents for a project.
    pub async fn list(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<DocumentResponse>> {
        let (docs, total) = tokio::try_join!(
            DocumentRepo::list_by_project(db, tenant_id, project_id, offset, limit),
            DocumentRepo::count_by_project(db, tenant_id, project_id),
        )?;

        Ok(PaginatedResponse {
            data: docs.into_iter().map(Into::into).collect(),
            total,
            offset,
            limit,
        })
    }

    /// Get a single document.
    pub async fn get(
        db: &PgPool,
        tenant_id: Uuid,
        document_id: Uuid,
    ) -> AppResult<DocumentResponse> {
        let doc = DocumentRepo::get_by_id(db, tenant_id, document_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Document not found".to_string(),
            })?;

        Ok(doc.into())
    }
}
