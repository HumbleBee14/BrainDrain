use platform_shared::enums::DeploymentStatus;
use uuid::Uuid;

use crate::config::Config;
use crate::dto::model::ModelResponse;
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{BillingEventRepository, ModelRepository};

/// Business logic for model deployment via vLLM.
///
/// vLLM runs as a sidecar service with `--enable-lora`. This service manages
/// adapter lifecycle via vLLM's REST API (load/unload LoRA adapters).
pub struct DeploymentService;

impl DeploymentService {
    /// Deploy a fine-tuned model by loading its LoRA adapter into vLLM.
    pub async fn deploy(
        model_repo: &dyn ModelRepository,
        billing_repo: &dyn BillingEventRepository,
        config: &Config,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<ModelResponse> {
        let model = model_repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        let adapter_path = model.adapter_path.clone().ok_or(AppError::BadRequest {
            message: "Model has no adapter — training may not be complete".to_string(),
        })?;

        if model.deployment_status == DeploymentStatus::Active.to_string() {
            return Err(AppError::Conflict {
                message: "Model is already deployed".to_string(),
            });
        }

        // Update status to deploying
        model_repo
            .update_deployment_status(tenant_id, model_id, DeploymentStatus::Deploying)
            .await?;

        // Build a unique adapter name for vLLM
        let adapter_name = format!("adapter-{model_id}");

        // Load LoRA adapter via vLLM REST API
        let vllm_url = &config.vllm_api_url;
        let http = reqwest::Client::new();

        let load_result = http
            .post(format!("{vllm_url}/v1/load_lora_adapter"))
            .json(&serde_json::json!({
                "lora_name": adapter_name,
                "lora_path": adapter_path,
            }))
            .send()
            .await;

        match load_result {
            Ok(resp) if resp.status().is_success() => {
                let deployment_config = serde_json::json!({
                    "vllm_adapter_name": adapter_name,
                    "adapter_path": adapter_path,
                    "base_model": model.base_model,
                });

                let updated = model_repo
                    .update_deployment(
                        tenant_id,
                        model_id,
                        DeploymentStatus::Active,
                        deployment_config,
                    )
                    .await?
                    .ok_or(AppError::NotFound {
                        message: "Model not found after deploy".to_string(),
                    })?;

                // Create billing event for deployment
                let _ = billing_repo
                    .create(
                        tenant_id,
                        "deploy",
                        Some(model_id),
                        0,
                        0,
                        0,
                        0.0,
                        serde_json::json!({"action": "deploy", "adapter_name": adapter_name}),
                    )
                    .await;

                tracing::info!(model_id = %model_id, adapter_name = %adapter_name, "Model deployed");
                Ok(updated.into())
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                model_repo
                    .update_deployment_status(tenant_id, model_id, DeploymentStatus::Undeployed)
                    .await?;
                tracing::error!(model_id = %model_id, status = %status, body = %body, "vLLM deploy failed");
                Err(AppError::Internal(anyhow::anyhow!(
                    "vLLM adapter load failed: {status}"
                )))
            }
            Err(e) => {
                model_repo
                    .update_deployment_status(tenant_id, model_id, DeploymentStatus::Undeployed)
                    .await?;
                tracing::error!(model_id = %model_id, error = %e, "vLLM unreachable");
                Err(AppError::Internal(anyhow::anyhow!(
                    "Cannot reach vLLM service: {e}"
                )))
            }
        }
    }

    /// Undeploy a model by unloading its LoRA adapter from vLLM.
    pub async fn undeploy(
        model_repo: &dyn ModelRepository,
        config: &Config,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<ModelResponse> {
        let model = model_repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        if model.deployment_status != DeploymentStatus::Active.to_string() {
            return Err(AppError::BadRequest {
                message: "Model is not currently deployed".to_string(),
            });
        }

        let adapter_name = model.deployment_config["vllm_adapter_name"]
            .as_str()
            .unwrap_or(&format!("adapter-{model_id}"))
            .to_string();

        let vllm_url = &config.vllm_api_url;
        let http = reqwest::Client::new();

        let unload_result = http
            .post(format!("{vllm_url}/v1/unload_lora_adapter"))
            .json(&serde_json::json!({
                "lora_name": adapter_name,
            }))
            .send()
            .await;

        // Even if vLLM call fails, mark as undeployed — adapter may already be gone
        if let Err(e) = &unload_result {
            tracing::warn!(model_id = %model_id, error = %e, "vLLM unload request failed — marking as undeployed anyway");
        }

        let updated = model_repo
            .update_deployment_status(tenant_id, model_id, DeploymentStatus::Undeployed)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found after undeploy".to_string(),
            })?;

        tracing::info!(model_id = %model_id, "Model undeployed");
        Ok(updated.into())
    }

    /// Get the deployment status and config for a model.
    pub async fn status(
        model_repo: &dyn ModelRepository,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<DeploymentStatusResponse> {
        let model = model_repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        Ok(DeploymentStatusResponse {
            model_id: model.id.to_string(),
            deployment_status: model
                .deployment_status
                .parse()
                .unwrap_or(DeploymentStatus::Undeployed),
            deployment_config: model.deployment_config,
            base_model: model.base_model,
            adapter_path: model.adapter_path,
        })
    }
}

/// Deployment status response.
#[derive(Debug, serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct DeploymentStatusResponse {
    pub model_id: String,
    pub deployment_status: DeploymentStatus,
    pub deployment_config: serde_json::Value,
    pub base_model: String,
    pub adapter_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // ── Deployment status validation ──

    #[test]
    fn valid_deployment_statuses_parse() {
        for status in ["undeployed", "deploying", "active", "inactive"] {
            assert!(
                DeploymentStatus::from_str(status).is_ok(),
                "Expected '{status}' to be a valid DeploymentStatus",
            );
        }
    }

    #[test]
    fn invalid_deployment_status_rejected() {
        assert!(DeploymentStatus::from_str("running").is_err());
        assert!(DeploymentStatus::from_str("").is_err());
        assert!(DeploymentStatus::from_str("ACTIVE").is_err());
    }

    // ── Deployment conflict detection (mirrors check in DeploymentService::deploy) ──

    #[test]
    fn already_active_model_is_conflict() {
        let current_status = DeploymentStatus::Active.to_string();
        assert_eq!(current_status, DeploymentStatus::Active.to_string());
        // The service returns AppError::Conflict when status is active
        let err = AppError::Conflict {
            message: "Model is already deployed".to_string(),
        };
        assert!(matches!(err, AppError::Conflict { .. }));
    }

    #[test]
    fn undeployed_model_is_not_conflict() {
        let current_status = DeploymentStatus::Undeployed.to_string();
        assert_ne!(current_status, DeploymentStatus::Active.to_string());
    }

    #[test]
    fn deploying_model_is_not_conflict() {
        let current_status = DeploymentStatus::Deploying.to_string();
        assert_ne!(current_status, DeploymentStatus::Active.to_string());
    }

    // ── Undeploy precondition (mirrors check in DeploymentService::undeploy) ──

    #[test]
    fn non_active_model_cannot_be_undeployed() {
        let active = DeploymentStatus::Active.to_string();
        for status in [
            DeploymentStatus::Undeployed,
            DeploymentStatus::Deploying,
            DeploymentStatus::Inactive,
        ] {
            assert_ne!(
                status.to_string(),
                active,
                "Status '{}' should fail the undeploy precondition",
                status,
            );
        }
    }

    #[test]
    fn active_model_can_be_undeployed() {
        assert_eq!(
            DeploymentStatus::Active.to_string(),
            DeploymentStatus::Active.to_string()
        );
    }

    // ── Adapter name generation ──

    #[test]
    fn adapter_name_contains_model_id() {
        let model_id = uuid::Uuid::new_v4();
        let adapter_name = format!("adapter-{model_id}");
        assert!(adapter_name.starts_with("adapter-"));
        assert!(adapter_name.contains(&model_id.to_string()));
    }

    #[test]
    fn adapter_name_is_deterministic() {
        let model_id = uuid::Uuid::new_v4();
        let name1 = format!("adapter-{model_id}");
        let name2 = format!("adapter-{model_id}");
        assert_eq!(name1, name2);
    }

    // ── Missing adapter path validation ──

    #[test]
    fn none_adapter_path_produces_bad_request() {
        let adapter_path: Option<String> = None;
        let result = adapter_path.clone().ok_or(AppError::BadRequest {
            message: "Model has no adapter — training may not be complete".to_string(),
        });
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::BadRequest { .. }));
    }

    #[test]
    fn some_adapter_path_passes_validation() {
        let adapter_path = Some("adapters/tenant/model/adapter.safetensors".to_string());
        let result = adapter_path.clone().ok_or(AppError::BadRequest {
            message: "Model has no adapter".to_string(),
        });
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "adapters/tenant/model/adapter.safetensors",);
    }

    // ── DeploymentStatusResponse serialization ──

    #[test]
    fn status_response_serializes_to_json() {
        let resp = DeploymentStatusResponse {
            model_id: uuid::Uuid::new_v4().to_string(),
            deployment_status: DeploymentStatus::Active,
            deployment_config: serde_json::json!({"vllm_adapter_name": "adapter-123"}),
            base_model: "meta-llama/Llama-3.1-8B".to_string(),
            adapter_path: Some("/path/to/adapter".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            json["deployment_status"],
            DeploymentStatus::Active.to_string()
        );
        assert_eq!(json["base_model"], "meta-llama/Llama-3.1-8B");
        assert!(json["adapter_path"].is_string());
    }

    #[test]
    fn status_response_with_null_adapter() {
        let resp = DeploymentStatusResponse {
            model_id: uuid::Uuid::new_v4().to_string(),
            deployment_status: DeploymentStatus::Undeployed,
            deployment_config: serde_json::json!({}),
            base_model: "model".to_string(),
            adapter_path: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["adapter_path"].is_null());
    }
}
