use bytes::Bytes;
use uuid::Uuid;

use platform_shared::enums::DatasetStatus;
use platform_shared::s3_paths;
use platform_storage::ObjectStorage;

use crate::dto::common::PaginatedResponse;
use crate::dto::dataset::{DatasetImportResponse, DatasetImportRowError, DatasetResponse};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{DatasetRepository, ProjectRepository};
use crate::services::jsonl_import;

/// Fraction of imported rows kept for training; the remainder is the
/// validation split. Mirrors the 90/10 split used by generated datasets.
const IMPORT_TRAIN_FRACTION: f64 = 0.9;

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

    /// Approve a dataset for training.
    /// Only datasets in `ReviewPending` status can be approved.
    pub async fn approve(
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

        let current_status: DatasetStatus =
            dataset.status.parse().unwrap_or(DatasetStatus::Generating);

        if current_status != DatasetStatus::ReviewPending {
            return Err(AppError::BadRequest {
                message: format!(
                    "Only datasets in 'review_pending' status can be approved. Current status: {}",
                    dataset.status
                ),
            });
        }

        let updated = repo
            .update_status(tenant_id, dataset_id, DatasetStatus::Approved)
            .await?
            .ok_or(AppError::Internal(anyhow::anyhow!(
                "Failed to update dataset status"
            )))?;

        Ok(updated.into())
    }

    /// Reject a dataset, archiving it so it cannot be used for training.
    /// Only datasets in `ReviewPending` status can be rejected.
    pub async fn reject(
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

        let current_status: DatasetStatus =
            dataset.status.parse().unwrap_or(DatasetStatus::Generating);

        if current_status != DatasetStatus::ReviewPending {
            return Err(AppError::BadRequest {
                message: format!(
                    "Only datasets in 'review_pending' status can be rejected. Current status: {}",
                    dataset.status
                ),
            });
        }

        let updated = repo
            .update_status(tenant_id, dataset_id, DatasetStatus::Archived)
            .await?
            .ok_or(AppError::Internal(anyhow::anyhow!(
                "Failed to update dataset status"
            )))?;

        Ok(updated.into())
    }

    /// Import an OpenAI-format chat JSONL dataset.
    ///
    /// Parses and validates every row (per-row errors, not a whole-file
    /// failure), splits the accepted rows into train/val, writes both splits to
    /// storage at the standard dataset keys, and creates a `review_pending`
    /// dataset row — so an imported dataset goes through the same approve/reject
    /// review as a generated one. Rejects the whole file only when no row is
    /// valid (nothing honest to store).
    #[allow(clippy::too_many_arguments)]
    pub async fn import_openai_jsonl(
        dataset_repo: &dyn DatasetRepository,
        project_repo: &dyn ProjectRepository,
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
        project_id: Uuid,
        name: &str,
        content: &str,
    ) -> AppResult<DatasetImportResponse> {
        // Scope the import to a project the caller owns.
        project_repo
            .get_by_id(tenant_id, project_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Project not found".to_string(),
            })?;

        let parsed = jsonl_import::parse_openai_jsonl(content);

        if parsed.records.is_empty() {
            let detail = parsed
                .errors
                .first()
                .map(|e| format!(" First error (line {}): {}", e.line, e.error))
                .unwrap_or_default();
            return Err(AppError::BadRequest {
                message: format!(
                    "No valid rows found in the uploaded file: {} row(s) seen, all rejected.{detail}",
                    parsed.total_rows
                ),
            });
        }

        let total = parsed.records.len();
        let dataset = Self::store_records_as_dataset(
            dataset_repo,
            storage,
            tenant_id,
            project_id,
            name,
            &parsed.records,
            serde_json::json!({
                "source": "openai_import",
                "rejected_rows": parsed.rejected_rows,
                "tool_records": parsed.tool_records,
            }),
        )
        .await?;

        let errors = parsed
            .errors
            .into_iter()
            .map(|e| DatasetImportRowError {
                line: e.line as u32,
                error: e.error,
            })
            .collect();

        Ok(DatasetImportResponse {
            dataset: dataset.into(),
            imported_rows: total as u32,
            rejected_rows: parsed.rejected_rows as u32,
            errors,
        })
    }

    /// Persist chat records as a new dataset: 90/10 train/val split, JSONL
    /// objects in storage, and a `review_pending` dataset row. `extra_stats`
    /// entries (e.g. source, provenance) are merged into the stats JSONB
    /// alongside the computed pair counts.
    pub async fn store_records_as_dataset(
        dataset_repo: &dyn DatasetRepository,
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
        project_id: Uuid,
        name: &str,
        records: &[serde_json::Value],
        extra_stats: serde_json::Value,
    ) -> AppResult<platform_db::models::Dataset> {
        let dataset_id = Uuid::new_v4();
        let dataset_key = s3_paths::dataset_path(tenant_id, project_id, dataset_id);

        // 90/10 train/val split, mirroring generated datasets.
        let total = records.len();
        let split_idx = ((total as f64 * IMPORT_TRAIN_FRACTION) as usize).max(1);
        let (train, val) = records.split_at(split_idx.min(total));

        let train_bytes = jsonl_bytes(train);
        let mut size_bytes = train_bytes.len() as i64;
        storage
            .put(&dataset_key, train_bytes, "application/jsonl")
            .await
            .map_err(AppError::Storage)?;

        if !val.is_empty() {
            let val_key = dataset_key.replace(".jsonl", "_val.jsonl");
            let val_bytes = jsonl_bytes(val);
            size_bytes += val_bytes.len() as i64;
            storage
                .put(&val_key, val_bytes, "application/jsonl")
                .await
                .map_err(AppError::Storage)?;
        }

        let mut stats = serde_json::json!({
            "total_pairs": total,
            "train_pairs": train.len(),
            "val_pairs": val.len(),
        });
        if let (Some(stats_map), Some(extra_map)) = (stats.as_object_mut(), extra_stats.as_object())
        {
            for (key, value) in extra_map {
                stats_map.insert(key.clone(), value.clone());
            }
        }

        // The row is what makes the uploaded objects reachable; if it can't be
        // created, remove them so a failed import leaves nothing behind.
        let created = dataset_repo
            .create_imported(
                tenant_id,
                project_id,
                dataset_id,
                name.to_string(),
                dataset_key.clone(),
                total as i32,
                stats,
                size_bytes,
            )
            .await;
        if created.is_err() {
            let val_key = dataset_key.replace(".jsonl", "_val.jsonl");
            for key in [dataset_key.as_str(), val_key.as_str()] {
                if let Err(e) = storage.delete(key).await {
                    tracing::warn!(key, error = %e, "failed to clean up orphaned dataset object");
                }
            }
        }
        created
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

/// Serialize records as a JSONL byte buffer (one compact JSON object per line).
fn jsonl_bytes(records: &[serde_json::Value]) -> Bytes {
    let mut buf = String::new();
    for (i, record) in records.iter().enumerate() {
        if i > 0 {
            buf.push('\n');
        }
        // Records originate from serde_json::Value, so serialization cannot fail.
        buf.push_str(&record.to_string());
    }
    Bytes::from(buf)
}
