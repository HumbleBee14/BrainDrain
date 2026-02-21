use uuid::Uuid;

use platform_shared::s3_paths;
use platform_storage::ObjectStorage;

use crate::dto::common::PaginatedResponse;
use crate::dto::dataset::DatasetResponse;
use crate::error::{AppError, AppResult};
use crate::repositories::traits::DatasetRepository;

/// Business logic for dataset operations.
pub struct DatasetService;

impl DatasetService {
    /// List datasets for a project.
    pub async fn list(
        repo: &dyn DatasetRepository,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<DatasetResponse>> {
        let (datasets, total) = tokio::try_join!(
            repo.list_by_project(tenant_id, project_id, offset, limit),
            repo.count_by_project(tenant_id, project_id),
        )?;

        Ok(PaginatedResponse {
            data: datasets.into_iter().map(Into::into).collect(),
            total,
            offset,
            limit,
        })
    }

    /// Get a single dataset.
    pub async fn get(
        repo: &dyn DatasetRepository,
        tenant_id: Uuid,
        dataset_id: Uuid,
    ) -> AppResult<DatasetResponse> {
        let dataset = repo
            .get_by_id(tenant_id, dataset_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Dataset not found".to_string(),
            })?;

        Ok(dataset.into())
    }

    /// Get a preview of dataset contents (first N rows from the JSONL file).
    pub async fn preview(
        repo: &dyn DatasetRepository,
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
        dataset_id: Uuid,
        max_rows: usize,
    ) -> AppResult<Vec<serde_json::Value>> {
        let dataset = repo
            .get_by_id(tenant_id, dataset_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Dataset not found".to_string(),
            })?;

        let storage_path = dataset.storage_path.ok_or(AppError::BadRequest {
            message: "Dataset has no stored file yet".to_string(),
        })?;

        let data = storage
            .get(&storage_path)
            .await
            .map_err(AppError::Storage)?;

        let content = String::from_utf8_lossy(&data);
        let rows: Vec<serde_json::Value> = content
            .lines()
            .take(max_rows)
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        Ok(rows)
    }

    /// Get a presigned URL for the parsed content of a document.
    pub async fn get_parsed_url(
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
        project_id: Uuid,
        document_id: Uuid,
    ) -> AppResult<String> {
        let path = s3_paths::parsed_path(tenant_id, project_id, document_id);

        let exists = storage.exists(&path).await.map_err(AppError::Storage)?;

        if !exists {
            return Err(AppError::NotFound {
                message: "Parsed content not found for this document".to_string(),
            });
        }

        let url = storage
            .presigned_url(&path, 3600)
            .await
            .map_err(AppError::Storage)?;

        Ok(url)
    }
}
