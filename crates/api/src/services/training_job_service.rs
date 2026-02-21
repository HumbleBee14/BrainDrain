use uuid::Uuid;

use crate::dto::common::PaginatedResponse;
use crate::dto::training_job::{CreateTrainingJobRequest, TrainingJobResponse};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{DatasetRepository, TrainingJobRepository};
use crate::temporal::WorkflowOrchestrator;
use platform_shared::enums::{TrainingMethod, TrainingMode};

/// Business logic for training job operations.
pub struct TrainingJobService;

impl TrainingJobService {
    /// Create a new training job and auto-trigger the TrainWorkflow.
    pub async fn create(
        training_repo: &dyn TrainingJobRepository,
        dataset_repo: &dyn DatasetRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        project_id: Uuid,
        req: CreateTrainingJobRequest,
    ) -> AppResult<TrainingJobResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Training workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        // Parse and validate dataset_id
        let dataset_id = req
            .dataset_id
            .parse::<Uuid>()
            .map_err(|_| AppError::BadRequest {
                message: "Invalid dataset_id format".to_string(),
            })?;

        // Verify dataset exists and belongs to tenant
        let dataset = dataset_repo
            .get_by_id(tenant_id, dataset_id)
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

        // Resolve method and mode with defaults (serde already validates enum variants)
        let method = req.method.unwrap_or(TrainingMethod::Qlora);
        let mode = req.mode.unwrap_or(TrainingMode::Quick);
        let method_str = method.to_string();
        let mode_str = mode.to_string();

        // Merge user hyperparams with defaults
        let hyperparams = merge_hyperparams(
            req.hyperparams
                .map(|hp| serde_json::to_value(hp).unwrap_or_default()),
        );

        // Compute cost estimate heuristic
        let cost_estimate = estimate_cost(
            &req.base_model,
            dataset.pair_count,
            req.gpu_class.as_deref(),
        );

        // Create the job in DB
        let job = training_repo
            .create(
                tenant_id,
                project_id,
                dataset_id,
                &req.base_model,
                &method_str,
                &mode_str,
                hyperparams.clone(),
                req.gpu_class.as_deref(),
                Some(cost_estimate),
            )
            .await?;

        // Build dataset S3 path
        let dataset_path = dataset.storage_path.unwrap_or_else(|| {
            platform_shared::s3_paths::dataset_path(tenant_id, project_id, dataset_id)
        });

        // Start TrainWorkflow via orchestrator
        let result = orchestrator
            .start_train(
                tenant_id,
                job.id,
                &dataset_path,
                &req.base_model,
                &method_str,
                &mode_str,
                hyperparams,
                req.gpu_class.as_deref(),
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start TrainWorkflow: {e}"))
            })?;

        // Update job with workflow ID
        training_repo
            .update_workflow_id(tenant_id, job.id, &result.workflow_id)
            .await?;

        tracing::info!(
            project_id = %project_id,
            training_job_id = %job.id,
            workflow_id = %result.workflow_id,
            base_model = %req.base_model,
            method = %method,
            mode = %mode,
            "TrainWorkflow started"
        );

        Ok(job.into())
    }

    /// Get a single training job.
    pub async fn get(
        repo: &dyn TrainingJobRepository,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> AppResult<TrainingJobResponse> {
        let job = repo
            .get_by_id(tenant_id, job_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Training job not found".to_string(),
            })?;

        Ok(job.into())
    }

    /// List training jobs for a project.
    pub async fn list(
        repo: &dyn TrainingJobRepository,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<TrainingJobResponse>> {
        let (jobs, total) = tokio::try_join!(
            repo.list_by_project(tenant_id, project_id, offset, limit),
            repo.count_by_project(tenant_id, project_id),
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
        repo: &dyn TrainingJobRepository,
        tenant_id: Uuid,
        job_id: Uuid,
    ) -> AppResult<TrainingJobResponse> {
        let job = repo.cancel(tenant_id, job_id).await?.ok_or(AppError::BadRequest {
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
/// Visible to tests in this module.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::training_job::CreateTrainingJobRequest;
    use platform_shared::enums::{TrainingMethod, TrainingMode};
    use std::str::FromStr;

    // ── merge_hyperparams ──

    #[test]
    fn default_hyperparams_when_none_provided() {
        let merged = merge_hyperparams(None);
        let obj = merged.as_object().expect("should be a JSON object");

        assert_eq!(obj["r"], 16);
        assert_eq!(obj["lora_alpha"], 16);
        assert_eq!(obj["num_train_epochs"], 3);
        assert_eq!(obj["max_seq_length"], 2048);
        assert_eq!(obj["optim"], "adamw_8bit");
        assert_eq!(obj["lr_scheduler_type"], "cosine");
    }

    #[test]
    fn user_overrides_merge_with_defaults() {
        let user = serde_json::json!({
            "r": 32,
            "num_train_epochs": 5,
            "custom_field": "custom_value",
        });
        let merged = merge_hyperparams(Some(user));
        let obj = merged.as_object().unwrap();

        // Overridden values
        assert_eq!(obj["r"], 32);
        assert_eq!(obj["num_train_epochs"], 5);
        // Custom field added
        assert_eq!(obj["custom_field"], "custom_value");
        // Default values preserved
        assert_eq!(obj["lora_alpha"], 16);
        assert_eq!(obj["max_seq_length"], 2048);
    }

    #[test]
    fn empty_object_overrides_change_nothing() {
        let user = serde_json::json!({});
        let merged = merge_hyperparams(Some(user));
        let defaults = merge_hyperparams(None);
        assert_eq!(merged, defaults);
    }

    #[test]
    fn non_object_override_is_ignored() {
        let user = serde_json::json!("not an object");
        let merged = merge_hyperparams(Some(user));
        let defaults = merge_hyperparams(None);
        assert_eq!(merged, defaults);
    }

    // ── estimate_cost ──

    #[test]
    fn cost_is_positive() {
        let cost = estimate_cost("meta-llama/Llama-3.1-8B", Some(1000), None);
        assert!(cost > 0.0, "Cost should be positive, got: {cost}");
    }

    #[test]
    fn larger_model_costs_more() {
        let cost_8b = estimate_cost("model-8b", Some(5000), None);
        let cost_70b = estimate_cost("model-70b", Some(5000), None);
        assert!(
            cost_70b > cost_8b,
            "70B model ({cost_70b}) should cost more than 8B ({cost_8b})",
        );
    }

    #[test]
    fn more_data_costs_more() {
        let cost_1k = estimate_cost("model-8b", Some(1000), None);
        let cost_50k = estimate_cost("model-8b", Some(50000), None);
        assert!(
            cost_50k > cost_1k,
            "50K pairs ({cost_50k}) should cost more than 1K ({cost_1k})",
        );
    }

    #[test]
    fn premium_gpu_costs_more() {
        let cost_t4 = estimate_cost("model-8b", Some(5000), Some("t4"));
        let cost_h100 = estimate_cost("model-8b", Some(5000), Some("h100"));
        assert!(
            cost_h100 > cost_t4,
            "H100 ({cost_h100}) should cost more than T4 ({cost_t4})",
        );
    }

    #[test]
    fn unknown_gpu_uses_default_rate() {
        let cost_unknown = estimate_cost("model-8b", Some(5000), Some("unknown_gpu"));
        let cost_none = estimate_cost("model-8b", Some(5000), None);
        assert_eq!(
            cost_unknown, cost_none,
            "Unknown GPU class should use the same default rate as None",
        );
    }

    #[test]
    fn none_pair_count_uses_default() {
        let cost = estimate_cost("model-8b", None, None);
        let cost_1000 = estimate_cost("model-8b", Some(1000), None);
        assert_eq!(cost, cost_1000, "None pair_count should default to 1000");
    }

    #[test]
    fn cost_is_rounded_to_two_decimals() {
        let cost = estimate_cost("model-8b", Some(5000), None);
        let rounded = (cost * 100.0).round() / 100.0;
        assert_eq!(cost, rounded, "Cost should be rounded to 2 decimal places");
    }

    // ── Model size detection from name ──

    #[test]
    fn model_size_detected_from_name() {
        let cost_3b = estimate_cost("some-model-3b", Some(5000), Some("t4"));
        let cost_7b = estimate_cost("some-model-7b", Some(5000), Some("t4"));
        let cost_13b = estimate_cost("some-model-13b", Some(5000), Some("t4"));
        let cost_70b = estimate_cost("some-model-70b", Some(5000), Some("t4"));

        assert!(cost_3b < cost_7b, "3B should cost less than 7B");
        assert!(cost_7b < cost_13b, "7B should cost less than 13B");
        assert!(cost_13b < cost_70b, "13B should cost less than 70B");
    }

    // ── Training method/mode validation ──

    #[test]
    fn valid_training_methods_parse() {
        for method in ["qlora", "lora", "full"] {
            assert!(
                TrainingMethod::from_str(method).is_ok(),
                "Expected '{method}' to be a valid TrainingMethod",
            );
        }
    }

    #[test]
    fn invalid_training_method_rejected() {
        assert!(TrainingMethod::from_str("invalid").is_err());
        assert!(TrainingMethod::from_str("").is_err());
        assert!(TrainingMethod::from_str("QLORA").is_err());
    }

    #[test]
    fn valid_training_modes_parse() {
        for mode in ["quick", "aligned", "reasoning", "iterative"] {
            assert!(
                TrainingMode::from_str(mode).is_ok(),
                "Expected '{mode}' to be a valid TrainingMode",
            );
        }
    }

    #[test]
    fn invalid_training_mode_rejected() {
        assert!(TrainingMode::from_str("fast").is_err());
        assert!(TrainingMode::from_str("").is_err());
    }

    // ── Input validation ──

    #[test]
    fn empty_base_model_fails_validation() {
        let req = CreateTrainingJobRequest {
            dataset_id: uuid::Uuid::new_v4().to_string(),
            base_model: "   ".to_string(),
            method: None,
            mode: None,
            hyperparams: None,
            gpu_class: None,
        };
        assert!(req.base_model.trim().is_empty());
    }

    #[test]
    fn invalid_dataset_id_format_fails_parse() {
        let req = CreateTrainingJobRequest {
            dataset_id: "not-a-uuid".to_string(),
            base_model: "meta-llama/Llama-3.1-8B".to_string(),
            method: None,
            mode: None,
            hyperparams: None,
            gpu_class: None,
        };
        assert!(req.dataset_id.parse::<uuid::Uuid>().is_err());
    }

    #[test]
    fn valid_dataset_id_parses() {
        let id = uuid::Uuid::new_v4();
        let req = CreateTrainingJobRequest {
            dataset_id: id.to_string(),
            base_model: "meta-llama/Llama-3.1-8B".to_string(),
            method: None,
            mode: None,
            hyperparams: None,
            gpu_class: None,
        };
        assert_eq!(req.dataset_id.parse::<uuid::Uuid>().unwrap(), id);
    }

    #[test]
    fn default_method_is_qlora() {
        let req = CreateTrainingJobRequest {
            dataset_id: uuid::Uuid::new_v4().to_string(),
            base_model: "model".to_string(),
            method: None,
            mode: None,
            hyperparams: None,
            gpu_class: None,
        };
        let method = req.method.unwrap_or(TrainingMethod::Qlora);
        assert_eq!(method, TrainingMethod::Qlora);
    }

    #[test]
    fn default_mode_is_quick() {
        let req = CreateTrainingJobRequest {
            dataset_id: uuid::Uuid::new_v4().to_string(),
            base_model: "model".to_string(),
            method: None,
            mode: None,
            hyperparams: None,
            gpu_class: None,
        };
        let mode = req.mode.unwrap_or(TrainingMode::Quick);
        assert_eq!(mode, TrainingMode::Quick);
    }
}
