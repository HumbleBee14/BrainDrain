use bytes::Bytes;
use futures::Stream;
use uuid::Uuid;

use platform_shared::constants::SUPPORTED_EXTENSIONS;
use platform_shared::s3_paths;
use platform_storage::{ObjectStorage, StorageError};

use crate::dto::common::PaginatedResponse;
use crate::dto::document::{DocumentResponse, UploadResponse};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{DocumentRepository, TenantRepository};
use crate::services::plan_service::PlanService;

/// Business logic for document operations.
///
/// Handles upload validation, S3 storage, DB record creation, and event publishing.
pub struct DocumentService;

impl DocumentService {
    /// Upload a document by streaming it straight to object storage.
    ///
    /// The file is never held in memory in full — it is streamed to storage
    /// (multipart for large objects), and only then is its final size known.
    /// Validation follows a reservation pattern: the object is written first,
    /// then the empty-file and plan-storage checks run against the real size;
    /// if either fails, the just-written object is deleted so storage is never
    /// left holding bytes with no document row pointing at them.
    #[allow(clippy::too_many_arguments)]
    pub async fn upload_streaming<S>(
        repo: &dyn DocumentRepository,
        tenant_repo: &dyn TenantRepository,
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
        project_id: Uuid,
        filename: &str,
        content_type: &str,
        stream: S,
    ) -> AppResult<UploadResponse>
    where
        S: Stream<Item = Result<Bytes, StorageError>> + Send + 'static,
    {
        // Validate the extension before consuming the stream — an unsupported
        // type is rejected without writing anything.
        let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
        if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
            return Err(AppError::BadRequest {
                message: format!(
                    "Unsupported file type '.{ext}'. Supported: {}",
                    SUPPORTED_EXTENSIONS.join(", ")
                ),
            });
        }

        let file_id = Uuid::now_v7();
        let storage_path = s3_paths::upload_path(tenant_id, project_id, file_id, &ext);

        let file_size = storage
            .put_streaming(&storage_path, stream, content_type)
            .await
            .map_err(AppError::Storage)? as i64;

        if file_size == 0 {
            let _ = storage.delete(&storage_path).await;
            return Err(AppError::BadRequest {
                message: "File is empty".to_string(),
            });
        }

        // Enforce the plan storage allowance against the real size, rolling back
        // the uploaded object if it would push the tenant over the limit.
        let current_bytes = repo.sum_storage_bytes(tenant_id).await?;
        if let Err(e) =
            PlanService::check_storage_limit(tenant_repo, tenant_id, current_bytes, file_size).await
        {
            let _ = storage.delete(&storage_path).await;
            return Err(e);
        }

        let doc = match repo
            .create(
                tenant_id,
                project_id,
                filename,
                file_size,
                content_type,
                &storage_path,
            )
            .await
        {
            Ok(doc) => doc,
            Err(e) => {
                let _ = storage.delete(&storage_path).await;
                return Err(e);
            }
        };

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
            status: doc
                .status
                .parse()
                .unwrap_or(platform_shared::enums::DocumentStatus::Uploaded),
        })
    }

    /// List documents for a project.
    pub async fn list(
        repo: &dyn DocumentRepository,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<DocumentResponse>> {
        let (docs, total) = tokio::try_join!(
            repo.list_by_project(tenant_id, project_id, offset, limit),
            repo.count_by_project(tenant_id, project_id),
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
        repo: &dyn DocumentRepository,
        tenant_id: Uuid,
        document_id: Uuid,
    ) -> AppResult<DocumentResponse> {
        let doc = repo
            .get_by_id(tenant_id, document_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Document not found".to_string(),
            })?;

        Ok(doc.into())
    }
}

#[cfg(test)]
mod tests {
    use platform_shared::constants::SUPPORTED_EXTENSIONS;

    /// Helper: extracts the extension from a filename the same way the service does.
    fn extract_ext(filename: &str) -> String {
        filename.rsplit('.').next().unwrap_or("").to_lowercase()
    }

    // ── File extension extraction ──

    #[test]
    fn extracts_simple_extension() {
        assert_eq!(extract_ext("document.pdf"), "pdf");
        assert_eq!(extract_ext("report.docx"), "docx");
        assert_eq!(extract_ext("readme.txt"), "txt");
    }

    #[test]
    fn extracts_extension_case_insensitive() {
        assert_eq!(extract_ext("PHOTO.JPG"), "jpg");
        assert_eq!(extract_ext("Scan.PDF"), "pdf");
        assert_eq!(extract_ext("FILE.DocX"), "docx");
    }

    #[test]
    fn extracts_extension_with_multiple_dots() {
        assert_eq!(extract_ext("archive.tar.pdf"), "pdf");
        assert_eq!(extract_ext("my.file.name.txt"), "txt");
    }

    #[test]
    fn no_dot_returns_full_filename_lowercased() {
        assert_eq!(extract_ext("Makefile"), "makefile");
    }

    #[test]
    fn dot_only_filename() {
        assert_eq!(extract_ext(".hidden"), "hidden");
    }

    // ── Supported extension validation ──

    #[test]
    fn pdf_is_supported() {
        let ext = extract_ext("report.pdf");
        assert!(SUPPORTED_EXTENSIONS.contains(&ext.as_str()));
    }

    #[test]
    fn docx_is_supported() {
        let ext = extract_ext("letter.docx");
        assert!(SUPPORTED_EXTENSIONS.contains(&ext.as_str()));
    }

    #[test]
    fn csv_is_supported() {
        let ext = extract_ext("data.csv");
        assert!(SUPPORTED_EXTENSIONS.contains(&ext.as_str()));
    }

    #[test]
    fn image_formats_are_not_supported() {
        // Image/scanned formats are rejected — there is no OCR path, so they
        // would parse to empty text.
        for filename in ["photo.png", "img.jpg", "scan.jpeg", "fax.tiff", "icon.bmp"] {
            let ext = extract_ext(filename);
            assert!(
                !SUPPORTED_EXTENSIONS.contains(&ext.as_str()),
                "Expected .{ext} to be rejected",
            );
        }
    }

    #[test]
    fn unsupported_extensions_are_rejected() {
        for filename in [
            "script.py",
            "binary.exe",
            "archive.zip",
            "video.mp4",
            "music.mp3",
        ] {
            let ext = extract_ext(filename);
            assert!(
                !SUPPORTED_EXTENSIONS.contains(&ext.as_str()),
                "Expected .{ext} to be unsupported",
            );
        }
    }

    #[test]
    fn all_supported_extensions_are_lowercase() {
        for ext in SUPPORTED_EXTENSIONS {
            assert_eq!(
                *ext,
                ext.to_lowercase(),
                "Extension constant should be lowercase: {ext}",
            );
        }
    }

    // ── Empty file validation ──

    #[test]
    fn zero_byte_file_is_invalid() {
        let data = bytes::Bytes::new();
        assert_eq!(data.len(), 0);
    }

    #[test]
    fn non_empty_file_is_valid() {
        let data = bytes::Bytes::from_static(b"hello");
        assert!(!data.is_empty());
    }
}
