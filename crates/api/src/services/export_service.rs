use platform_storage::ObjectStorage;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dto::export::{ExportResponse, OllamaExportResponse, VALID_QUANT_TYPES};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{ExportRepository, ModelRepository};
use crate::services::deployment_service::DeploymentService;
use crate::services::ollama_modelfile;
use crate::temporal::{TraceContext, WorkflowOrchestrator};

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
        trace_ctx: TraceContext,
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
        let result = match orchestrator
            .start_export(
                tenant_id,
                model_id,
                export.id,
                &adapter_path,
                &model.base_model,
                quant_type,
                trace_ctx,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // Mark export as failed so it doesn't stay orphaned as "pending"
                let _ = export_repo
                    .update_status(
                        tenant_id,
                        export.id,
                        "failed",
                        None,
                        None,
                        Some(&format!("Workflow start failed: {e}")),
                    )
                    .await;
                return Err(AppError::Internal(anyhow::anyhow!(
                    "Failed to start ExportWorkflow: {e}"
                )));
            }
        };

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
        let export =
            export_repo
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

        let storage_path = export
            .storage_path
            .ok_or(AppError::Internal(anyhow::anyhow!(
                "Completed export has no storage path"
            )))?;

        let url = storage
            .presigned_url(&storage_path, 3600)
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to generate download URL: {e}"))
            })?;

        let filename = format!(
            "model-{}-{}.gguf",
            &export.model_id.to_string()[..8],
            export.quant_type
        );

        Ok((url, export.file_size_bytes, filename))
    }

    /// Build the one-click "run locally with Ollama" recipe for a completed
    /// export: a presigned GGUF download URL, a Modelfile, and the `ollama`
    /// commands. The Modelfile carries the base-model family's stop tokens and
    /// the trained system prompt so local generation matches how the model was
    /// served.
    pub async fn ollama_recipe(
        state: &AppState,
        tenant_id: Uuid,
        export_id: Uuid,
    ) -> AppResult<OllamaExportResponse> {
        let export = state
            .export_repo()
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

        let storage_path = export
            .storage_path
            .ok_or(AppError::Internal(anyhow::anyhow!(
                "Completed export has no storage path"
            )))?;

        let model = state
            .model_repo()
            .get_by_id(tenant_id, export.model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model for this export not found".to_string(),
            })?;

        // Same system prompt the deploy path serves under (train/serve parity).
        let system_prompt =
            DeploymentService::resolve_guide_system_prompt(state, tenant_id, &model).await;

        let download_url = state
            .storage()
            .presigned_url(&storage_path, 3600)
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to generate download URL: {e}"))
            })?;

        let filename = format!(
            "model-{}-{}.gguf",
            &export.model_id.to_string()[..8],
            export.quant_type
        );
        let model_name =
            ollama_modelfile::model_name(&export.model_id.to_string(), &export.quant_type);
        let modelfile =
            ollama_modelfile::build_modelfile(&model.base_model, &filename, &system_prompt);
        let instructions = ollama_modelfile::build_instructions(&model_name, &filename);

        Ok(OllamaExportResponse {
            model_name,
            filename,
            download_url,
            file_size_bytes: export.file_size_bytes,
            modelfile,
            instructions,
        })
    }
}
