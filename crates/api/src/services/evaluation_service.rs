use uuid::Uuid;

use crate::dto::common::PaginatedResponse;
use crate::dto::evaluation::{CreateEvaluationRequest, EvaluationResponse};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{
    DatasetRepository, EvaluationRepository, ModelRepository, TrainingJobRepository,
};
use crate::temporal::{TraceContext, WorkflowOrchestrator};

/// Business logic for evaluation operations.
pub struct EvaluationService;

impl EvaluationService {
    /// Create a new evaluation and start the EvaluateWorkflow.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        eval_repo: &dyn EvaluationRepository,
        model_repo: &dyn ModelRepository,
        training_repo: &dyn TrainingJobRepository,
        dataset_repo: &dyn DatasetRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        model_id: Uuid,
        req: CreateEvaluationRequest,
        trace_ctx: TraceContext,
    ) -> AppResult<EvaluationResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Evaluation workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        // Verify model exists and belongs to tenant
        let model = model_repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        let adapter_path = model.adapter_path.ok_or(AppError::BadRequest {
            message: "Model has no adapter — training may not be complete".to_string(),
        })?;

        // Get training job for dataset_path and base_model
        let training_job = training_repo
            .get_by_id(tenant_id, model.training_job_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Training job for this model not found".to_string(),
            })?;

        // Build dataset path from training job's dataset; keep the dataset
        // row for the evaluation context (teacher provenance).
        let dataset = dataset_repo
            .get_by_id(tenant_id, training_job.dataset_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Dataset for this model's training job not found".to_string(),
            })?;
        let dataset_path = dataset.storage_path.clone().unwrap_or_else(|| {
            platform_shared::s3_paths::dataset_path(
                tenant_id,
                training_job.project_id,
                training_job.dataset_id,
            )
        });

        // Create evaluation record
        let eval = eval_repo.create(tenant_id, model_id).await?;

        // Job context so mode-specific suites (teacher parity for distill
        // jobs) know what they are evaluating. The dataset config carries
        // credential-free teacher provenance only.
        // The artifacts prefix comes from the training run's own metrics rather
        // than its hyperparams: a high-fidelity run is the only thing that can
        // know where the teacher's distributions were stored, and the fidelity
        // metric has to read the same ones the student trained against.
        let job_config = serde_json::json!({
            "mode": training_job.mode,
            "method": training_job.method,
            "gpu_class": training_job.gpu_class,
            "teacher_artifacts_prefix": training_job
                .metrics
                .get("teacher_artifacts_prefix")
                .and_then(|value| value.as_str()),
        });

        // Start EvaluateWorkflow via orchestrator
        let result = orchestrator
            .start_evaluate(
                tenant_id,
                model_id,
                eval.id,
                &adapter_path,
                &model.base_model,
                &dataset_path,
                req.judge_model.as_deref(),
                req.judge_api_base.as_deref(),
                training_job.gpu_class.as_deref(),
                &training_job.mode,
                dataset.config,
                job_config,
                req.judge_thinking,
                trace_ctx,
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start EvaluateWorkflow: {e}"))
            })?;

        // Update evaluation with workflow ID
        eval_repo
            .update_workflow_id(tenant_id, eval.id, &result.workflow_id)
            .await?;

        tracing::info!(
            model_id = %model_id,
            evaluation_id = %eval.id,
            workflow_id = %result.workflow_id,
            "EvaluateWorkflow started"
        );

        Ok(eval.into())
    }

    /// Get a single evaluation.
    pub async fn get(
        repo: &dyn EvaluationRepository,
        tenant_id: Uuid,
        eval_id: Uuid,
    ) -> AppResult<EvaluationResponse> {
        let eval = repo
            .get_by_id(tenant_id, eval_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Evaluation not found".to_string(),
            })?;

        Ok(eval.into())
    }

    /// List evaluations for a model.
    pub async fn list(
        repo: &dyn EvaluationRepository,
        tenant_id: Uuid,
        model_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<EvaluationResponse>> {
        let (evals, total) = tokio::try_join!(
            repo.list_by_model(tenant_id, model_id, offset, limit),
            repo.count_by_model(tenant_id, model_id),
        )?;

        Ok(PaginatedResponse {
            data: evals.into_iter().map(Into::into).collect(),
            total,
            offset,
            limit,
        })
    }
}
