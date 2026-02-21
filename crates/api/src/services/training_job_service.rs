use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::common::PaginatedResponse;
use crate::dto::training_job::{CreateTrainingJobRequest, TrainingJobResponse};
use crate::error::{AppError, AppResult};
use crate::repositories::dataset_repo::DatasetRepo;
use crate::repositories::training_job_repo::TrainingJobRepo;
use crate::temporal::TemporalClient;
use platform_shared::enums::{TrainingMethod, TrainingMode};

/// Business logic for training job operations.
pub struct TrainingJobService;

impl TrainingJobService {
    /// Create a new training job and auto-trigger the TrainWorkflow.
    pub async fn create(
        db: &PgPool,
        temporal: Option<&TemporalClient>,
        tenant_id: Uuid,
        project_id: Uuid,
        req: CreateTrainingJobRequest,
    ) -> AppResult<TrainingJobResponse> {
        let temporal = temporal.ok_or(AppError::BadRequest {
            message: "Training workflows are not available (Temporal not configured)".to_string(),
        })?;

        // Parse and validate dataset_id
        let dataset_id = req
            .dataset_id
            .parse::<Uuid>()
            .map_err(|_| AppError::BadRequest {
                message: "Invalid dataset_id format".to_string(),
            })?;

        // Verify dataset exists and belongs to tenant
        let dataset = DatasetRepo::get_by_id(db, tenant_id, dataset_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Dataset not found".to_string(),
            })?;

        // Validate base_model
        if req.base_model.trim().is_empty() {
            return Err(AppError::BadRequest {
                message: "base_model is required".to_string(),
            });
        }

        // Validate method via shared enum (auto-synced with TypeScript/Python)
        let method_str = req.method.as_deref().unwrap_or("qlora");
        let _method: TrainingMethod = method_str.parse().map_err(|_| AppError::BadRequest {
            message: format!(
                "Invalid method '{method_str}'. Must be {}",
                ["qlora", "lora", "full"].join(", ")
            ),
        })?;
        let method = method_str;

        // Validate mode via shared enum (auto-synced with TypeScript/Python)
        let mode_str = req.mode.as_deref().unwrap_or("quick");
        let _mode: TrainingMode = mode_str.parse().map_err(|_| AppError::BadRequest {
            message: format!(
                "Invalid mode '{mode_str}'. Must be {}",
                ["quick", "aligned", "reasoning", "iterative"].join(", ")
            ),
        })?;
        let mode = mode_str;

        // Merge user hyperparams with defaults
        let hyperparams = merge_hyperparams(req.hyperparams);

        // Compute cost estimate heuristic
        let cost_estimate = estimate_cost(
            &req.base_model,
            dataset.pair_count,
            req.gpu_class.as_deref(),
        );

        // Create the job in DB
        let job = TrainingJobRepo::create(
            db,
            tenant_id,
            project_id,
            dataset_id,
            &req.base_model,
            method,
            mode,
            hyperparams.clone(),
            req.gpu_class.as_deref(),
            Some(cost_estimate),
        )
        .await?;

        // Build dataset S3 path
        let dataset_path = dataset.storage_path.unwrap_or_else(|| {
            platform_shared::s3_paths::dataset_path(tenant_id, project_id, dataset_id)
        });

        // Start TrainWorkflow via Temporal
        let result = temporal
            .start_train(
                tenant_id,
                job.id,
                &dataset_path,
                &req.base_model,
                method,
                mode,
                hyperparams,
                req.gpu_class.as_deref(),
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start TrainWorkflow: {e}"))
            })?;

        // Update job with workflow ID
        TrainingJobRepo::update_workflow_id(db, tenant_id, job.id, &result.workflow_id).await?;

        tracing::info!(
            project_id = %project_id,
            training_job_id = %job.id,
            workflow_id = %result.workflow_id,
            base_model = %req.base_model,
            method = method,
            mode = mode,
            "TrainWorkflow started"
        );

        Ok(job.into())
    }

    /// Get a single training job.
    pub async fn get(db: &PgPool, tenant_id: Uuid, job_id: Uuid) -> AppResult<TrainingJobResponse> {
        let job = TrainingJobRepo::get_by_id(db, tenant_id, job_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Training job not found".to_string(),
            })?;

        Ok(job.into())
    }

    /// List training jobs for a project.
    pub async fn list(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<TrainingJobResponse>> {
        let (jobs, total) = tokio::try_join!(
            TrainingJobRepo::list_by_project(db, tenant_id, project_id, offset, limit),
            TrainingJobRepo::count_by_project(db, tenant_id, project_id),
        )?;

        Ok(PaginatedResponse {
            data: jobs.into_iter().map(Into::into).collect(),
            total,
            offset,
            limit,
        })
    }

    /// Cancel a training job.
    pub async fn cancel(
        db: &PgPool,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> AppResult<TrainingJobResponse> {
        let job = TrainingJobRepo::cancel(db, tenant_id, job_id)
            .await?
            .ok_or(AppError::BadRequest {
            message:
                "Cannot cancel: job not found or not in a cancellable state (pending/cost_approval)"
                    .to_string(),
        })?;

        tracing::info!(training_job_id = %job_id, "Training job cancelled");

        Ok(job.into())
    }
}

/// Merge user-provided hyperparams with smart defaults.
fn merge_hyperparams(user_params: Option<serde_json::Value>) -> serde_json::Value {
    let mut defaults = serde_json::json!({
        "r": 16,
        "lora_alpha": 16,
        "lora_dropout": 0,
        "target_modules": [
            "q_proj", "k_proj", "v_proj", "o_proj",
            "gate_proj", "up_proj", "down_proj"
        ],
        "learning_rate": 2e-4,
        "per_device_train_batch_size": 2,
        "gradient_accumulation_steps": 4,
        "num_train_epochs": 3,
        "warmup_steps": 10,
        "optim": "adamw_8bit",
        "lr_scheduler_type": "cosine",
        "max_seq_length": 2048,
    });

    if let Some(overrides) = user_params
        && let (Some(base), Some(over)) = (defaults.as_object_mut(), overrides.as_object())
    {
        for (key, value) in over {
            base.insert(key.clone(), value.clone());
        }
    }

    defaults
}

/// Estimate training cost based on model size, dataset size, and GPU class.
fn estimate_cost(base_model: &str, pair_count: Option<i32>, gpu_class: Option<&str>) -> f64 {
    // Parse approximate parameter count from model name
    let model_lower = base_model.to_lowercase();
    let params_b = if model_lower.contains("70b") {
        70.0
    } else if model_lower.contains("13b") {
        13.0
    } else if model_lower.contains("8b") || model_lower.contains("7b") {
        8.0
    } else if model_lower.contains("3b") {
        3.0
    } else if model_lower.contains("1b") || model_lower.contains("1.5b") {
        1.5
    } else {
        7.0 // default assumption
    };

    let pairs = pair_count.unwrap_or(1000) as f64;

    // GPU hourly rate from shared constants
    let gpu_rate = gpu_class
        .and_then(|cls| {
            platform_shared::constants::GPU_HOURLY_RATES
                .iter()
                .find(|(name, _)| *name == cls)
                .map(|(_, rate)| *rate)
        })
        .unwrap_or(platform_shared::constants::GPU_DEFAULT_HOURLY_RATE);

    // Rough estimate: larger models and more data take longer
    let estimated_hours = (params_b / 7.0) * (pairs / 5000.0).max(0.5) * 0.5;
    (estimated_hours * gpu_rate * 100.0).round() / 100.0
}
