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

    /// Hard-delete a document: its S3 objects (upload + parsed) and its DB row.
    ///
    /// The row is the index into storage, so a stranded row pointing at deleted
    /// objects is worse than an orphaned object a later sweep can reclaim.
    /// S3 deletes are therefore best-effort — a real delete error is logged but
    /// does not block removal of the row. A never-parsed document has no parsed
    /// object; deleting a missing key is a no-op success.
    pub async fn delete(
        repo: &dyn DocumentRepository,
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
        document_id: Uuid,
    ) -> AppResult<()> {
        let doc = repo
            .get_by_id(tenant_id, document_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Document not found".to_string(),
            })?;

        let parsed = s3_paths::parsed_path(tenant_id, doc.project_id, document_id);
        for key in [doc.storage_path.as_str(), parsed.as_str()] {
            if let Err(e) = storage.delete(key).await {
                tracing::warn!(
                    document_id = %document_id,
                    key = key,
                    error = %e,
                    "Failed to delete document object; deleting row anyway"
                );
            }
        }

        repo.delete(tenant_id, document_id).await?;

        tracing::info!(
            document_id = %document_id,
            tenant_id = %tenant_id,
            "Document deleted"
        );

        Ok(())
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

    // ── Delete ──

    use std::collections::HashMap;
    use std::sync::Mutex;

    use platform_db::models::Document;
    use platform_shared::s3_paths;
    use platform_storage::memory::InMemoryStorage;
    use platform_storage::{ObjectStorage, StorageError};
    use uuid::Uuid;

    use super::DocumentService;
    use crate::error::{AppError, AppResult};
    use crate::repositories::traits::{BoxFuture, DocumentRepository};

    fn make_doc(tenant_id: Uuid, project_id: Uuid, storage_path: &str) -> Document {
        Document {
            id: Uuid::now_v7(),
            tenant_id,
            project_id,
            filename: "f.pdf".to_string(),
            file_size: 10,
            mime_type: "application/pdf".to_string(),
            storage_path: storage_path.to_string(),
            status: "uploaded".to_string(),
            parse_quality: None,
            page_count: None,
            language: None,
            domain: None,
            metadata: serde_json::Value::Null,
            error_message: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    /// Fake repo backed by an in-memory map. Only the methods the delete path
    /// exercises are implemented; the rest panic if a test misuses them.
    struct FakeDocumentRepo {
        docs: Mutex<HashMap<Uuid, Document>>,
    }

    impl FakeDocumentRepo {
        fn new(docs: Vec<Document>) -> Self {
            Self {
                docs: Mutex::new(docs.into_iter().map(|d| (d.id, d)).collect()),
            }
        }

        fn contains(&self, id: Uuid) -> bool {
            self.docs.lock().unwrap().contains_key(&id)
        }
    }

    impl DocumentRepository for FakeDocumentRepo {
        fn get_by_id(
            &self,
            tenant_id: Uuid,
            document_id: Uuid,
        ) -> BoxFuture<'_, AppResult<Option<Document>>> {
            let doc = self
                .docs
                .lock()
                .unwrap()
                .get(&document_id)
                .filter(|d| d.tenant_id == tenant_id)
                .cloned();
            Box::pin(async move { Ok(doc) })
        }

        fn delete(&self, tenant_id: Uuid, document_id: Uuid) -> BoxFuture<'_, AppResult<bool>> {
            let removed = {
                let mut docs = self.docs.lock().unwrap();
                match docs.get(&document_id) {
                    Some(d) if d.tenant_id == tenant_id => docs.remove(&document_id).is_some(),
                    _ => false,
                }
            };
            Box::pin(async move { Ok(removed) })
        }

        fn create(
            &self,
            _tenant_id: Uuid,
            _project_id: Uuid,
            _filename: &str,
            _file_size: i64,
            _mime_type: &str,
            _storage_path: &str,
        ) -> BoxFuture<'_, AppResult<Document>> {
            unimplemented!()
        }

        fn list_by_project(
            &self,
            _tenant_id: Uuid,
            _project_id: Uuid,
            _offset: i64,
            _limit: i64,
        ) -> BoxFuture<'_, AppResult<Vec<Document>>> {
            unimplemented!()
        }

        fn list_by_status(
            &self,
            _tenant_id: Uuid,
            _project_id: Uuid,
            _status: platform_shared::enums::DocumentStatus,
        ) -> BoxFuture<'_, AppResult<Vec<Document>>> {
            unimplemented!()
        }

        fn count_by_status(
            &self,
            _tenant_id: Uuid,
            _project_id: Uuid,
            _status: platform_shared::enums::DocumentStatus,
        ) -> BoxFuture<'_, AppResult<i64>> {
            unimplemented!()
        }

        fn update_status(
            &self,
            _tenant_id: Uuid,
            _document_id: Uuid,
            _status: platform_shared::enums::DocumentStatus,
            _error_message: Option<&str>,
        ) -> BoxFuture<'_, AppResult<bool>> {
            unimplemented!()
        }

        fn count_by_project(
            &self,
            _tenant_id: Uuid,
            _project_id: Uuid,
        ) -> BoxFuture<'_, AppResult<i64>> {
            unimplemented!()
        }

        fn count_by_tenant(&self, _tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
            unimplemented!()
        }

        fn sum_storage_bytes(&self, _tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
            unimplemented!()
        }
    }

    /// Storage whose `delete` always fails — used to prove the row is removed
    /// even when object deletion errors.
    struct FailingDeleteStorage;

    impl ObjectStorage for FailingDeleteStorage {
        async fn delete(&self, _key: &str) -> Result<(), StorageError> {
            Err(StorageError::DeleteFailed("boom".to_string()))
        }

        async fn delete_prefix(&self, _prefix: &str) -> Result<usize, StorageError> {
            unimplemented!()
        }

        async fn put(
            &self,
            _key: &str,
            _data: bytes::Bytes,
            _content_type: &str,
        ) -> Result<(), StorageError> {
            unimplemented!()
        }

        async fn put_streaming<S>(
            &self,
            _key: &str,
            _stream: S,
            _content_type: &str,
        ) -> Result<u64, StorageError>
        where
            S: futures::Stream<Item = Result<bytes::Bytes, StorageError>> + Send + 'static,
        {
            unimplemented!()
        }

        async fn get(&self, _key: &str) -> Result<bytes::Bytes, StorageError> {
            unimplemented!()
        }

        async fn exists(&self, _key: &str) -> Result<bool, StorageError> {
            unimplemented!()
        }

        async fn list_prefix(
            &self,
            _prefix: &str,
        ) -> Result<Vec<platform_storage::ObjectMeta>, StorageError> {
            unimplemented!()
        }

        async fn presigned_url(
            &self,
            _key: &str,
            _expiry_secs: u64,
        ) -> Result<String, StorageError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn delete_removes_row_and_both_objects() {
        let tenant = Uuid::now_v7();
        let project = Uuid::now_v7();
        let doc = make_doc(tenant, project, "uploads/t/p/f.pdf");
        let doc_id = doc.id;
        let parsed = s3_paths::parsed_path(tenant, project, doc_id);

        let storage = InMemoryStorage::new();
        storage
            .put(
                &doc.storage_path,
                bytes::Bytes::from_static(b"raw"),
                "application/pdf",
            )
            .await
            .unwrap();
        storage
            .put(
                &parsed,
                bytes::Bytes::from_static(b"{}"),
                "application/json",
            )
            .await
            .unwrap();

        let repo = FakeDocumentRepo::new(vec![doc]);

        DocumentService::delete(&repo, &storage, tenant, doc_id)
            .await
            .unwrap();

        assert!(!repo.contains(doc_id), "row should be gone");
        assert!(storage.is_empty().await, "both objects should be gone");
    }

    #[tokio::test]
    async fn delete_succeeds_when_never_parsed() {
        let tenant = Uuid::now_v7();
        let project = Uuid::now_v7();
        let doc = make_doc(tenant, project, "uploads/t/p/f.pdf");
        let doc_id = doc.id;

        let storage = InMemoryStorage::new();
        storage
            .put(
                &doc.storage_path,
                bytes::Bytes::from_static(b"raw"),
                "application/pdf",
            )
            .await
            .unwrap();

        let repo = FakeDocumentRepo::new(vec![doc]);

        DocumentService::delete(&repo, &storage, tenant, doc_id)
            .await
            .unwrap();

        assert!(!repo.contains(doc_id));
        assert!(storage.is_empty().await);
    }

    #[tokio::test]
    async fn delete_missing_document_is_not_found() {
        let repo = FakeDocumentRepo::new(vec![]);
        let storage = InMemoryStorage::new();

        let err = DocumentService::delete(&repo, &storage, Uuid::now_v7(), Uuid::now_v7())
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::NotFound { .. }));
    }

    #[tokio::test]
    async fn delete_removes_row_even_when_storage_delete_fails() {
        let tenant = Uuid::now_v7();
        let project = Uuid::now_v7();
        let doc = make_doc(tenant, project, "uploads/t/p/f.pdf");
        let doc_id = doc.id;

        let repo = FakeDocumentRepo::new(vec![doc]);

        DocumentService::delete(&repo, &FailingDeleteStorage, tenant, doc_id)
            .await
            .unwrap();

        assert!(
            !repo.contains(doc_id),
            "row should be deleted despite S3 failure"
        );
    }
}
