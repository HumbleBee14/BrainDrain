use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::dto::model::ModelResponse;
use crate::error::{AppError, AppResult};
use crate::repositories::billing_event_repo::BillingEventRepo;
use crate::repositories::model_repo::ModelRepo;

/// Business logic for model deployment via vLLM.
///
/// vLLM runs as a sidecar service with `--enable-lora`. This service manages
/// adapter lifecycle via vLLM's REST API (load/unload LoRA adapters).
pub struct DeploymentService;

impl DeploymentService {
    /// Deploy a fine-tuned model by loading its LoRA adapter into vLLM.
    pub async fn deploy(
        db: &PgPool,
        config: &Config,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<ModelResponse> {
        let model =
            ModelRepo::get_by_id(db, tenant_id, model_id)
                .await?
                .ok_or(AppError::NotFound {
                    message: "Model not found".to_string(),
                })?;

        let adapter_path = model.adapter_path.clone().ok_or(AppError::BadRequest {
            message: "Model has no adapter — training may not be complete".to_string(),
        })?;

        if model.deployment_status == "active" {
            return Err(AppError::Conflict {
                message: "Model is already deployed".to_string(),
            });
        }

        // Update status to deploying
        ModelRepo::update_deployment_status(db, tenant_id, model_id, "deploying").await?;

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

                let updated = ModelRepo::update_deployment(
                    db,
                    tenant_id,
                    model_id,
                    "active",
                    deployment_config,
                )
                .await?
                .ok_or(AppError::NotFound {
                    message: "Model not found after deploy".to_string(),
                })?;

                // Create billing event for deployment
                let _ = BillingEventRepo::create(
                    db,
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
                ModelRepo::update_deployment_status(db, tenant_id, model_id, "undeployed").await?;
                tracing::error!(model_id = %model_id, status = %status, body = %body, "vLLM deploy failed");
                Err(AppError::Internal(anyhow::anyhow!(
                    "vLLM adapter load failed: {status}"
                )))
            }
            Err(e) => {
                ModelRepo::update_deployment_status(db, tenant_id, model_id, "undeployed").await?;
                tracing::error!(model_id = %model_id, error = %e, "vLLM unreachable");
                Err(AppError::Internal(anyhow::anyhow!(
                    "Cannot reach vLLM service: {e}"
                )))
            }
        }
    }

    /// Undeploy a model by unloading its LoRA adapter from vLLM.
    pub async fn undeploy(
        db: &PgPool,
        config: &Config,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<ModelResponse> {
        let model =
            ModelRepo::get_by_id(db, tenant_id, model_id)
                .await?
                .ok_or(AppError::NotFound {
                    message: "Model not found".to_string(),
                })?;

        if model.deployment_status != "active" {
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

        let updated = ModelRepo::update_deployment_status(db, tenant_id, model_id, "undeployed")
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found after undeploy".to_string(),
            })?;

        tracing::info!(model_id = %model_id, "Model undeployed");
        Ok(updated.into())
    }

    /// Get the deployment status and config for a model.
    pub async fn status(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<DeploymentStatusResponse> {
        let model =
            ModelRepo::get_by_id(db, tenant_id, model_id)
                .await?
                .ok_or(AppError::NotFound {
                    message: "Model not found".to_string(),
                })?;

        Ok(DeploymentStatusResponse {
            model_id: model.id.to_string(),
            deployment_status: model.deployment_status,
            deployment_config: model.deployment_config,
            base_model: model.base_model,
            adapter_path: model.adapter_path,
        })
    }
}

/// Deployment status response.
#[derive(Debug, serde::Serialize)]
pub struct DeploymentStatusResponse {
    pub model_id: String,
    pub deployment_status: String,
    pub deployment_config: serde_json::Value,
    pub base_model: String,
    pub adapter_path: Option<String>,
}
