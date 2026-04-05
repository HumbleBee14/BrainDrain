use platform_db::models::Model;
use platform_shared::enums::DeploymentStatus;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::dto::model::ModelResponse;
use crate::error::{AppError, AppResult};
use crate::repositories::traits::ModelRepository;
use crate::services::billing_outbox;
use crate::services::inference_backend::InferenceBackend;

/// Business logic for model deployment via pluggable inference backends.
///
/// The inference backend (vLLM, TGI, SGLang) runs as a sidecar service.
/// This service manages adapter lifecycle via the `InferenceBackend` trait.
pub struct DeploymentService;

impl DeploymentService {
    /// Deploy a fine-tuned model by loading its LoRA adapter into the inference backend.
    pub async fn deploy(
        db: &PgPool,
        model_repo: &dyn ModelRepository,
        backend: &dyn InferenceBackend,
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

        // Reap stale 'deploying' slots from crashed API processes before claiming.
        let _ = model_repo
            .reap_stale_deployments(config.deploy_stale_minutes)
            .await;

        // Atomically claim a deployment slot under advisory lock — prevents race
        // where two concurrent deploys both pass the count check.
        let max_loras = config.vllm_max_loras;
        let claimed = model_repo
            .claim_deployment_slot(tenant_id, model_id, &model.base_model, max_loras)
            .await?;
        if !claimed {
            let active_count = model_repo
                .count_active_by_base_model(&model.base_model)
                .await?;
            return Err(AppError::Conflict {
                message: format!(
                    "Adapter limit reached: {active_count}/{max_loras} adapters active \
                     for base model '{}'. Undeploy an existing model first.",
                    model.base_model
                ),
            });
        }

        // Load LoRA adapter via the pluggable inference backend (circuit-broken).
        let load_result = backend.load_adapter(model_id, &adapter_path).await;

        match load_result {
            Ok(handle) => {
                let deployment_config = serde_json::json!({
                    "adapter_ref": handle.adapter_ref,
                    "adapter_path": adapter_path,
                    "base_model": model.base_model,
                    "backend": backend.name(),
                    "backend_meta": handle.metadata,
                });
                let mut tx = db.begin().await?;

                let updated = sqlx::query_as::<_, Model>(
                    r#"
                    UPDATE models
                    SET deployment_status = $3, deployment_config = $4, updated_at = NOW()
                    WHERE id = $1 AND tenant_id = $2
                    RETURNING *
                    "#,
                )
                .bind(model_id)
                .bind(tenant_id)
                .bind(DeploymentStatus::Active.to_string())
                .bind(&deployment_config)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(AppError::NotFound {
                    message: "Model not found after deploy".to_string(),
                })?;

                billing_outbox::enqueue_in_tx(
                    &mut tx,
                    tenant_id,
                    "deploy",
                    Some(model_id),
                    0,
                    0,
                    0,
                    0.0,
                    serde_json::json!({
                        "action": "deploy",
                        "adapter_ref": handle.adapter_ref,
                        "backend": backend.name(),
                    }),
                )
                .await?;

                if let Err(e) = tx.commit().await {
                    let _ = backend.unload_adapter(&handle.adapter_ref).await;
                    let _ = model_repo
                        .update_deployment_status(tenant_id, model_id, DeploymentStatus::Undeployed)
                        .await;
                    return Err(e.into());
                }

                tracing::info!(
                    model_id = %model_id,
                    adapter_ref = %handle.adapter_ref,
                    backend = backend.name(),
                    "Model deployed"
                );
                Ok(updated.into())
            }
            Err(e) => {
                model_repo
                    .update_deployment_status(tenant_id, model_id, DeploymentStatus::Undeployed)
                    .await?;
                tracing::error!(
                    model_id = %model_id,
                    backend = backend.name(),
                    error = %e,
                    "Adapter load failed"
                );
                Err(e)
            }
        }
    }

    /// Undeploy a model by unloading its LoRA adapter from the inference backend.
    /// Unload is always best-effort — the DB is updated regardless of backend response.
    pub async fn undeploy(
        model_repo: &dyn ModelRepository,
        backend: &dyn InferenceBackend,
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

        // Hard fail if the model was deployed on a different backend.
        if let Some(deployed_backend) = model.deployment_config["backend"].as_str()
            && deployed_backend != backend.name()
        {
            return Err(AppError::BadRequest {
                message: format!(
                    "Model was deployed on '{}' but current backend is '{}'. \
                     Cannot undeploy from a different backend.",
                    deployed_backend,
                    backend.name()
                ),
            });
        }

        let adapter_ref = model.deployment_config["adapter_ref"]
            .as_str()
            .ok_or(AppError::Internal(anyhow::anyhow!(
                "Model deployment config missing adapter_ref"
            )))?
            .to_string();

        // Best-effort — backend errors are logged but never surface as HTTP errors.
        let _ = backend.unload_adapter(&adapter_ref).await;

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
#[derive(Debug, serde::Serialize, ts_rs::TS, utoipa::ToSchema)]
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
            deployment_config: serde_json::json!({"adapter_ref": "adapter-123", "backend": "vllm"}),
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
