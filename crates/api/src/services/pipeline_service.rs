use platform_shared::enums::{
    DatasetStatus, DeploymentStatus, DocumentStatus, EvaluationStatus, TrainingJobStatus,
};
use uuid::Uuid;

use crate::dto::pipeline::{
    DatasetStatusCounts, DocumentStatusCounts, EvaluationStatusCounts, ModelStatusCounts,
    ProjectPipelineStatus, TrainingJobStatusCounts, TriggerFullPipelineResponse,
    TriggerParseResponse, TriggerRefineResponse,
};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{
    DatasetRepository, DocumentRepository, EvaluationRepository, ModelRepository,
    TrainingJobRepository,
};
use crate::temporal::WorkflowOrchestrator;

/// Business logic for pipeline orchestration.
///
/// Validates preconditions and triggers Temporal workflows.
pub struct PipelineService;

impl PipelineService {
    /// Trigger document parsing for all unparsed documents in a project.
    ///
    /// Finds documents with status "uploaded" and starts an IngestWorkflow.
    pub async fn trigger_parse(
        doc_repo: &dyn DocumentRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> AppResult<TriggerParseResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Pipeline workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        let docs = doc_repo
            .list_by_status(tenant_id, project_id, DocumentStatus::Uploaded)
            .await?;

        if docs.is_empty() {
            return Err(AppError::BadRequest {
                message: "No documents ready for parsing (status must be 'uploaded')".to_string(),
            });
        }

        let doc_ids: Vec<Uuid> = docs.iter().map(|d| d.id).collect();
        let doc_count = doc_ids.len();

        let result = orchestrator
            .start_ingest(tenant_id, project_id, doc_ids)
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start IngestWorkflow: {e}"))
            })?;

        tracing::info!(
            project_id = %project_id,
            workflow_id = %result.workflow_id,
            document_count = doc_count,
            "IngestWorkflow started"
        );

        Ok(TriggerParseResponse {
            workflow_id: result.workflow_id,
            document_count: doc_count,
        })
    }

    /// Trigger data refinement for parsed documents in a project.
    ///
    /// Finds documents with status "parsed" and starts a RefineWorkflow.
    pub async fn trigger_refine(
        doc_repo: &dyn DocumentRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        project_id: Uuid,
        task_type: &str,
        config: serde_json::Value,
    ) -> AppResult<TriggerRefineResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Pipeline workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        let docs = doc_repo
            .list_by_status(tenant_id, project_id, DocumentStatus::Parsed)
            .await?;

        if docs.is_empty() {
            return Err(AppError::BadRequest {
                message: "No parsed documents available for refinement".to_string(),
            });
        }

        let doc_ids: Vec<Uuid> = docs.iter().map(|d| d.id).collect();
        let doc_count = doc_ids.len();

        let result = orchestrator
            .start_refine(tenant_id, project_id, doc_ids, task_type, config)
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start RefineWorkflow: {e}"))
            })?;

        tracing::info!(
            project_id = %project_id,
            workflow_id = %result.workflow_id,
            document_count = doc_count,
            task_type = task_type,
            "RefineWorkflow started"
        );

        Ok(TriggerRefineResponse {
            workflow_id: result.workflow_id,
            document_count: doc_count,
        })
    }

    /// Trigger the full pipeline: ingest → refine → train → evaluate → (optional deploy).
    ///
    /// Starts a FullPipelineWorkflow for all uploaded documents in a project.
    pub async fn trigger_full_pipeline(
        doc_repo: &dyn DocumentRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        project_id: Uuid,
        task_type: &str,
        base_model: &str,
        training_config: serde_json::Value,
    ) -> AppResult<TriggerFullPipelineResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Pipeline workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        // Collect all documents that haven't failed (uploaded or parsed)
        let (uploaded, parsed) = tokio::try_join!(
            doc_repo.list_by_status(tenant_id, project_id, DocumentStatus::Uploaded),
            doc_repo.list_by_status(tenant_id, project_id, DocumentStatus::Parsed),
        )?;

        let mut doc_ids: Vec<Uuid> = uploaded.iter().map(|d| d.id).collect();
        doc_ids.extend(parsed.iter().map(|d| d.id));

        if doc_ids.is_empty() {
            return Err(AppError::BadRequest {
                message:
                    "No documents available for the pipeline (need uploaded or parsed documents)"
                        .to_string(),
            });
        }

        let doc_count = doc_ids.len();

        let result = orchestrator
            .start_full_pipeline(
                tenant_id,
                project_id,
                doc_ids,
                task_type,
                base_model,
                training_config,
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start FullPipelineWorkflow: {e}"))
            })?;

        tracing::info!(
            project_id = %project_id,
            workflow_id = %result.workflow_id,
            document_count = doc_count,
            base_model = base_model,
            "FullPipelineWorkflow started"
        );

        Ok(TriggerFullPipelineResponse {
            workflow_id: result.workflow_id,
            document_count: doc_count,
        })
    }

    /// Get aggregate pipeline status for a project.
    pub async fn get_status(
        doc_repo: &dyn DocumentRepository,
        dataset_repo: &dyn DatasetRepository,
        training_repo: &dyn TrainingJobRepository,
        model_repo: &dyn ModelRepository,
        eval_repo: &dyn EvaluationRepository,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> AppResult<ProjectPipelineStatus> {
        let (
            total_docs,
            uploaded,
            parsing,
            parsed,
            doc_failed,
            total_datasets,
            generating,
            review_pending,
            approved,
            total_jobs,
            jobs_pending,
            jobs_training,
            jobs_completed,
            jobs_failed,
            total_models,
            models_undeployed,
            models_active,
            total_evals,
            evals_running,
            evals_completed,
            evals_failed,
        ) = tokio::try_join!(
            doc_repo.count_by_project(tenant_id, project_id),
            doc_repo.count_by_status(tenant_id, project_id, DocumentStatus::Uploaded),
            doc_repo.count_by_status(tenant_id, project_id, DocumentStatus::Parsing),
            doc_repo.count_by_status(tenant_id, project_id, DocumentStatus::Parsed),
            doc_repo.count_by_status(tenant_id, project_id, DocumentStatus::Failed),
            dataset_repo.count_by_project(tenant_id, project_id),
            dataset_repo.count_by_status(tenant_id, project_id, DatasetStatus::Generating),
            dataset_repo.count_by_status(tenant_id, project_id, DatasetStatus::ReviewPending),
            dataset_repo.count_by_status(tenant_id, project_id, DatasetStatus::Approved),
            training_repo.count_by_project(tenant_id, project_id),
            training_repo.count_by_status(tenant_id, project_id, TrainingJobStatus::Pending),
            training_repo.count_by_status(tenant_id, project_id, TrainingJobStatus::Training),
            training_repo.count_by_status(tenant_id, project_id, TrainingJobStatus::Completed),
            training_repo.count_by_status(tenant_id, project_id, TrainingJobStatus::Failed),
            model_repo.count_by_project(tenant_id, project_id),
            model_repo.count_by_deployment_status(
                tenant_id,
                project_id,
                DeploymentStatus::Undeployed
            ),
            model_repo.count_by_deployment_status(tenant_id, project_id, DeploymentStatus::Active),
            eval_repo.count_by_project(tenant_id, project_id),
            eval_repo.count_by_project_status(tenant_id, project_id, EvaluationStatus::Running),
            eval_repo.count_by_project_status(tenant_id, project_id, EvaluationStatus::Completed),
            eval_repo.count_by_project_status(tenant_id, project_id, EvaluationStatus::Failed),
        )?;

        Ok(ProjectPipelineStatus {
            project_id: project_id.to_string(),
            documents: DocumentStatusCounts {
                total: total_docs,
                uploaded,
                parsing,
                parsed,
                failed: doc_failed,
            },
            datasets: DatasetStatusCounts {
                total: total_datasets,
                generating,
                review_pending,
                approved,
            },
            training_jobs: TrainingJobStatusCounts {
                total: total_jobs,
                pending: jobs_pending,
                training: jobs_training,
                completed: jobs_completed,
                failed: jobs_failed,
            },
            models: ModelStatusCounts {
                total: total_models,
                undeployed: models_undeployed,
                active: models_active,
            },
            evaluations: EvaluationStatusCounts {
                total: total_evals,
                running: evals_running,
                completed: evals_completed,
                failed: evals_failed,
            },
        })
    }
}
