use uuid::Uuid;

use crate::dto::export::{ExportResponse, VALID_QUANT_TYPES};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{ExportRepository, ModelRepository};
use crate::temporal::WorkflowOrchestrator;

/// Business logic for GGUF model export operations.
pub struct ExportService;

impl ExportService {
    /// Start a GGUF export: validate, create DB row, trigger Temporal workflow.
    pub async fn create(
        export_repo: &dyn ExportRepository,
        model_repo: &dyn ModelRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        model_id: Uuid,
        quant_type: &str,
    ) -> AppResult<ExportResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Export workflows are not available (orchestrator not configured)".to_string(),
        })?;

        // Validate quant type
        if !VALID_QUANT_TYPES.contains(&quant_type) {
            return Err(AppError::BadRequest {
                message: format!(
                    "Invalid quantization type '{}'. Valid: {}",
                    quant_type,
                    VALID_QUANT_TYPES.join(", ")
                ),
            });
        }

        // Verify model exists, belongs to tenant, and has an adapter
        let model = model_repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        let adapter_path = model.adapter_path.ok_or(AppError::BadRequest {
            message: "Model has no adapter — training may not be complete".to_string(),
        })?;

        // Check for existing pending/processing exports of the same quant type
        let existing = export_repo.list_by_model(tenant_id, model_id).await?;
        if existing.iter().any(|e| {
            e.quant_type == quant_type && (e.status == "pending" || e.status == "processing")
        }) {
            return Err(AppError::Conflict {
                message: format!(
                    "An export with quantization type '{}' is already in progress",
                    quant_type
                ),
            });
        }

        // Create export record
        let export = export_repo
            .create(tenant_id, model_id, "gguf", quant_type)
            .await?;

        // Trigger ExportWorkflow via Temporal
        let result = orchestrator
            .start_export(
                tenant_id,
                model_id,
                export.id,
                &adapter_path,
                &model.base_model,
                quant_type,
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start ExportWorkflow: {e}"))
            })?;

        tracing::info!(
            model_id = %model_id,
            export_id = %export.id,
            quant_type = %quant_type,
            workflow_id = %result.workflow_id,
            "ExportWorkflow started"
        );

        Ok(export.into())
    }

    /// List exports for a model.
    pub async fn list(
        export_repo: &dyn ExportRepository,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<Vec<ExportResponse>> {
        let exports = export_repo.list_by_model(tenant_id, model_id).await?;
        Ok(exports.into_iter().map(Into::into).collect())
    }

    /// Get a presigned download URL for a completed export.
    pub async fn download_url(
        export_repo: &dyn ExportRepository,
        storage: &impl platform_storage::ObjectStorage,
        tenant_id: Uuid,
        export_id: Uuid,
    ) -> AppResult<(String, Option<i64>, String)> {
        let export = export_repo
            .get_by_id(tenant_id, export_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Export not found".to_string(),
            })?;

        if export.status != "completed" {
            return Err(AppError::BadRequest {
                message: format!("Export is not ready (status: {})", export.status),
            });
        }

        let storage_path = export.storage_path.ok_or(AppError::Internal(
            anyhow::anyhow!("Completed export has no storage path"),
        ))?;

        let url = storage
            .presigned_url(&storage_path, 3600)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to generate download URL: {e}")))?;

        let filename = format!(
            "model-{}-{}.gguf",
            &export.model_id.to_string()[..8],
            export.quant_type
        );

        Ok((url, export.file_size_bytes, filename))
    }
}
