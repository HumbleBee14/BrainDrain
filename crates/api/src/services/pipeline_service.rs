use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::pipeline::{
    DatasetStatusCounts, DocumentStatusCounts, EvaluationStatusCounts, ModelStatusCounts,
    ProjectPipelineStatus, TrainingJobStatusCounts, TriggerParseResponse, TriggerRefineResponse,
};
use crate::error::{AppError, AppResult};
use crate::repositories::dataset_repo::DatasetRepo;
use crate::repositories::document_repo::DocumentRepo;
use crate::repositories::evaluation_repo::EvaluationRepo;
use crate::repositories::model_repo::ModelRepo;
use crate::repositories::training_job_repo::TrainingJobRepo;
use crate::temporal::TemporalClient;

/// Business logic for pipeline orchestration.
///
/// Validates preconditions and triggers Temporal workflows.
pub struct PipelineService;

impl PipelineService {
    /// Trigger document parsing for all unparsed documents in a project.
    ///
    /// Finds documents with status "uploaded" and starts an IngestWorkflow.
    pub async fn trigger_parse(
        db: &PgPool,
        temporal: Option<&TemporalClient>,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> AppResult<TriggerParseResponse> {
        let temporal = temporal.ok_or(AppError::BadRequest {
            message: "Pipeline workflows are not available (Temporal not configured)".to_string(),
        })?;

        let docs = DocumentRepo::list_by_status(db, tenant_id, project_id, "uploaded").await?;

        if docs.is_empty() {
            return Err(AppError::BadRequest {
                message: "No documents ready for parsing (status must be 'uploaded')".to_string(),
            });
        }

        let doc_ids: Vec<Uuid> = docs.iter().map(|d| d.id).collect();
        let doc_count = doc_ids.len();

        let result = temporal
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
        db: &PgPool,
        temporal: Option<&TemporalClient>,
        tenant_id: Uuid,
        project_id: Uuid,
        task_type: &str,
        config: serde_json::Value,
    ) -> AppResult<TriggerRefineResponse> {
        let temporal = temporal.ok_or(AppError::BadRequest {
            message: "Pipeline workflows are not available (Temporal not configured)".to_string(),
        })?;

        let docs = DocumentRepo::list_by_status(db, tenant_id, project_id, "parsed").await?;

        if docs.is_empty() {
            return Err(AppError::BadRequest {
                message: "No parsed documents available for refinement".to_string(),
            });
        }

        let doc_ids: Vec<Uuid> = docs.iter().map(|d| d.id).collect();
        let doc_count = doc_ids.len();

        let result = temporal
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

    /// Get aggregate pipeline status for a project.
    pub async fn get_status(
        db: &PgPool,
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
            DocumentRepo::count_by_project(db, tenant_id, project_id),
            DocumentRepo::count_by_status(db, tenant_id, project_id, "uploaded"),
            DocumentRepo::count_by_status(db, tenant_id, project_id, "parsing"),
            DocumentRepo::count_by_status(db, tenant_id, project_id, "parsed"),
            DocumentRepo::count_by_status(db, tenant_id, project_id, "failed"),
            DatasetRepo::count_by_project(db, tenant_id, project_id),
            DatasetRepo::count_by_status(db, tenant_id, project_id, "generating"),
            DatasetRepo::count_by_status(db, tenant_id, project_id, "review_pending"),
            DatasetRepo::count_by_status(db, tenant_id, project_id, "approved"),
            TrainingJobRepo::count_by_project(db, tenant_id, project_id),
            TrainingJobRepo::count_by_status(db, tenant_id, project_id, "pending"),
            TrainingJobRepo::count_by_status(db, tenant_id, project_id, "training"),
            TrainingJobRepo::count_by_status(db, tenant_id, project_id, "completed"),
            TrainingJobRepo::count_by_status(db, tenant_id, project_id, "failed"),
            ModelRepo::count_by_project(db, tenant_id, project_id),
            ModelRepo::count_by_deployment_status(db, tenant_id, project_id, "undeployed"),
            ModelRepo::count_by_deployment_status(db, tenant_id, project_id, "active"),
            EvaluationRepo::count_by_project(db, tenant_id, project_id),
            EvaluationRepo::count_by_project_status(db, tenant_id, project_id, "running"),
            EvaluationRepo::count_by_project_status(db, tenant_id, project_id, "completed"),
            EvaluationRepo::count_by_project_status(db, tenant_id, project_id, "failed"),
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
