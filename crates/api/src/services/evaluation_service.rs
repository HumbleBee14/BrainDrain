use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::common::PaginatedResponse;
use crate::dto::evaluation::{CreateEvaluationRequest, EvaluationResponse};
use crate::error::{AppError, AppResult};
use crate::repositories::evaluation_repo::EvaluationRepo;
use crate::repositories::model_repo::ModelRepo;
use crate::repositories::training_job_repo::TrainingJobRepo;
use crate::temporal::WorkflowOrchestrator;

/// Business logic for evaluation operations.
pub struct EvaluationService;

impl EvaluationService {
    /// Create a new evaluation and start the EvaluateWorkflow.
    pub async fn create(
        db: &PgPool,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        model_id: Uuid,
        req: CreateEvaluationRequest,
    ) -> AppResult<EvaluationResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Evaluation workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        // Verify model exists and belongs to tenant
        let model =
            ModelRepo::get_by_id(db, tenant_id, model_id)
                .await?
                .ok_or(AppError::NotFound {
                    message: "Model not found".to_string(),
                })?;

        let adapter_path = model.adapter_path.ok_or(AppError::BadRequest {
            message: "Model has no adapter — training may not be complete".to_string(),
        })?;

        // Get training job for dataset_path and base_model
        let training_job = TrainingJobRepo::get_by_id(db, tenant_id, model.training_job_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Training job for this model not found".to_string(),
            })?;

        // Build dataset path from training job's dataset
        let dataset_path = {
            let dataset = crate::repositories::dataset_repo::DatasetRepo::get_by_id(
                db,
                tenant_id,
                training_job.dataset_id,
            )
            .await?
            .ok_or(AppError::NotFound {
                message: "Dataset for this model's training job not found".to_string(),
            })?;

            dataset.storage_path.unwrap_or_else(|| {
                platform_shared::s3_paths::dataset_path(
                    tenant_id,
                    training_job.project_id,
                    training_job.dataset_id,
                )
            })
        };

        // Create evaluation record
        let eval = EvaluationRepo::create(db, tenant_id, model_id).await?;

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
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start EvaluateWorkflow: {e}"))
            })?;

        // Update evaluation with workflow ID
        EvaluationRepo::update_workflow_id(db, tenant_id, eval.id, &result.workflow_id).await?;

        tracing::info!(
            model_id = %model_id,
            evaluation_id = %eval.id,
            workflow_id = %result.workflow_id,
            "EvaluateWorkflow started"
        );

        Ok(eval.into())
    }

    /// Get a single evaluation.
    pub async fn get(db: &PgPool, tenant_id: Uuid, eval_id: Uuid) -> AppResult<EvaluationResponse> {
        let eval = EvaluationRepo::get_by_id(db, tenant_id, eval_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Evaluation not found".to_string(),
            })?;

        Ok(eval.into())
    }

    /// List evaluations for a model.
    pub async fn list(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<EvaluationResponse>> {
        let (evals, total) = tokio::try_join!(
            EvaluationRepo::list_by_model(db, tenant_id, model_id, offset, limit),
            EvaluationRepo::count_by_model(db, tenant_id, model_id),
        )?;

        Ok(PaginatedResponse {
            data: evals.into_iter().map(Into::into).collect(),
            total,
            offset,
            limit,
        })
    }
}
