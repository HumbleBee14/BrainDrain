use platform_db::models::Model;
use platform_shared::enums::DeploymentStatus;
use sqlx::PgPool;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dto::model::ModelResponse;
use crate::error::{AppError, AppResult};
use crate::services::billing_outbox;
use crate::services::feature_flags::{DEPLOYMENTS_MULTI_INSTANCE_ENABLED, FlagContext};
use crate::services::inference_backend::AdapterHandle;

/// Business logic for model deployment via pluggable inference backends.
pub struct DeploymentService;

impl DeploymentService {
    /// Deploy a fine-tuned model by loading its LoRA adapter into the inference backend.
    pub async fn deploy(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<ModelResponse> {
        let model = state
            .model_repo()
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        let adapter_path = model.adapter_path.clone().ok_or(AppError::BadRequest {
            message: "Model has no adapter - training may not be complete".to_string(),
        })?;

        if model.deployment_status == DeploymentStatus::Active.to_string() {
            return Err(AppError::Conflict {
                message: "Model is already deployed".to_string(),
            });
        }

        let multi_instance_enabled = state
            .feature_flags()
            .is_enabled(DEPLOYMENTS_MULTI_INSTANCE_ENABLED, &FlagContext::default());

        if multi_instance_enabled {
            Self::deploy_multi_instance(state, tenant_id, model_id, &model, &adapter_path).await
        } else {
            Self::deploy_single_instance(state, tenant_id, model_id, &model, &adapter_path).await
        }
    }

    pub async fn undeploy(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<ModelResponse> {
        let model = state
            .model_repo()
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

        let adapter_ref = model.deployment_config["adapter_ref"]
            .as_str()
            .ok_or(AppError::Internal(anyhow::anyhow!(
                "Model deployment config missing adapter_ref"
            )))?
            .to_string();

        let backend_name = model.deployment_config["backend"]
            .as_str()
            .ok_or(AppError::Internal(anyhow::anyhow!(
                "Model deployment config missing backend"
            )))?
            .to_string();

        // Resolve the backend for this model's assigned instance (or global).
        let (resolved_backend, instance_id) = if let Some(instance_id) = model.inference_instance_id
        {
            let instance = state
                .inference_instance_repo()
                .get_by_id(instance_id)
                .await?
                .ok_or(AppError::ServiceUnavailable {
                    message: "Assigned inference instance no longer exists".to_string(),
                })?;
            if instance.backend_type != backend_name {
                return Err(AppError::BadRequest {
                    message: format!(
                        "Model deployment config backend '{}' does not match assigned instance backend '{}'",
                        backend_name, instance.backend_type
                    ),
                });
            }
            let be = state
                .build_inference_backend_for_instance(&instance.backend_type, &instance.base_url);
            (be, Some(instance_id))
        } else {
            if backend_name != state.inference_backend().name() {
                return Err(AppError::BadRequest {
                    message: format!(
                        "Model was deployed on '{}' but current backend is '{}'. Cannot undeploy from a different backend.",
                        backend_name,
                        state.inference_backend().name(),
                    ),
                });
            }
            (state.inference_backend_arc(), None)
        };

        // IMPORTANT: Commit DB changes FIRST, then unload adapter best-effort.
        // If we unloaded first and crashed before commit, the model would stay
        // 'active' in DB with a dead adapter on the instance.
        let mut tx = state.db().begin().await?;
        let updated = sqlx::query_as::<_, Model>(
            r#"
            UPDATE models
            SET deployment_status = $3,
                inference_instance_id = NULL,
                deployment_config = '{}'::jsonb,
                updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(model_id)
        .bind(tenant_id)
        .bind(DeploymentStatus::Undeployed.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound {
            message: "Model not found after undeploy".to_string(),
        })?;

        if let Some(inst_id) = instance_id {
            sqlx::query(
                r#"
                UPDATE inference_instances
                SET active_adapter_count = GREATEST(active_adapter_count - 1, 0),
                    updated_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(inst_id)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        // Best-effort adapter unload AFTER DB commit.
        // If this fails, the adapter lingers until instance restart — acceptable.
        let _ = resolved_backend.unload_adapter(&adapter_ref).await;

        tracing::info!(model_id = %model_id, "Model undeployed");
        Ok(updated.into())
    }

    pub async fn status(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<DeploymentStatusResponse> {
        let model = state
            .model_repo()
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

    async fn deploy_single_instance(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
        model: &Model,
        adapter_path: &str,
    ) -> AppResult<ModelResponse> {
        let _ = state
            .model_repo()
            .reap_stale_deployments(state.config().deploy_stale_minutes)
            .await;

        let max_loras = state.config().vllm_max_loras;
        let claimed = state
            .model_repo()
            .claim_deployment_slot(tenant_id, model_id, &model.base_model, max_loras)
            .await?;
        if !claimed {
            let active_count = state
                .model_repo()
                .count_active_by_base_model(&model.base_model)
                .await?;
            return Err(AppError::Conflict {
                message: format!(
                    "Adapter limit reached: {active_count}/{max_loras} adapters active for base model '{}'. Undeploy an existing model first.",
                    model.base_model
                ),
            });
        }

        let backend = state.inference_backend();
        let handle = match backend.load_adapter(model_id, adapter_path).await {
            Ok(handle) => handle,
            Err(error) => {
                Self::reset_model_deployment(state.db(), tenant_id, model_id).await?;
                tracing::error!(model_id = %model_id, backend = backend.name(), error = %error, "Adapter load failed");
                return Err(error);
            }
        };

        let deployment_config = serde_json::json!({
            "adapter_ref": handle.adapter_ref,
            "adapter_path": adapter_path,
            "base_model": model.base_model,
            "backend": backend.name(),
            "backend_meta": handle.metadata,
        });

        match Self::finalize_deploy(
            state.db(),
            tenant_id,
            model_id,
            None,
            deployment_config,
            backend.name(),
            &handle,
        )
        .await
        {
            Ok(model) => Ok(model.into()),
            Err(error) => {
                let _ = backend.unload_adapter(&handle.adapter_ref).await;
                let _ = Self::reset_model_deployment(state.db(), tenant_id, model_id).await;
                Err(error)
            }
        }
    }

    async fn deploy_multi_instance(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
        model: &Model,
        adapter_path: &str,
    ) -> AppResult<ModelResponse> {
        let instance = state
            .inference_instance_repo()
            .claim_slot(&state.config().inference_backend_type, &model.base_model)
            .await?
            .ok_or(AppError::ServiceUnavailable {
                message: format!(
                    "No healthy ready inference instance with capacity for backend '{}' and base model '{}'",
                    state.config().inference_backend_type,
                    model.base_model
                ),
            })?;

        // Atomically claim the model for deployment. The WHERE clause
        // prevents two concurrent deploys from both succeeding — only the
        // first to set 'deploying' wins. The loser gets rows_affected=0
        // and releases its claimed instance slot.
        let claimed_row = sqlx::query(
            r#"
            UPDATE models
            SET deployment_status = 'deploying',
                inference_instance_id = $3,
                updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
              AND deployment_status NOT IN ('deploying', 'active')
            "#,
        )
        .bind(model_id)
        .bind(tenant_id)
        .bind(instance.id)
        .execute(state.db())
        .await?;

        if claimed_row.rows_affected() == 0 {
            let _ = state
                .inference_instance_repo()
                .release_slot(instance.id)
                .await;
            return Err(AppError::Conflict {
                message: "Model is already being deployed or is active".to_string(),
            });
        }

        let backend =
            state.build_inference_backend_for_instance(&instance.backend_type, &instance.base_url);

        let handle = match backend.load_adapter(model_id, adapter_path).await {
            Ok(handle) => handle,
            Err(error) => {
                let _ = state
                    .inference_instance_repo()
                    .release_slot(instance.id)
                    .await;
                let _ = Self::reset_model_deployment(state.db(), tenant_id, model_id).await;
                tracing::error!(
                    model_id = %model_id,
                    instance_id = %instance.id,
                    backend = backend.name(),
                    error = %error,
                    "Adapter load failed on assigned inference instance"
                );
                return Err(error);
            }
        };

        let deployment_config = serde_json::json!({
            "adapter_ref": handle.adapter_ref,
            "adapter_path": adapter_path,
            "base_model": model.base_model,
            "backend": backend.name(),
            "instance_id": instance.id.to_string(),
            "instance_name": instance.name,
            "instance_url": instance.base_url,
            "backend_meta": handle.metadata,
        });

        match Self::finalize_deploy(
            state.db(),
            tenant_id,
            model_id,
            Some(instance.id),
            deployment_config,
            backend.name(),
            &handle,
        )
        .await
        {
            Ok(model) => Ok(model.into()),
            Err(error) => {
                let _ = backend.unload_adapter(&handle.adapter_ref).await;
                let _ = state
                    .inference_instance_repo()
                    .release_slot(instance.id)
                    .await;
                let _ = Self::reset_model_deployment(state.db(), tenant_id, model_id).await;
                Err(error)
            }
        }
    }

    async fn finalize_deploy(
        db: &PgPool,
        tenant_id: Uuid,
        model_id: Uuid,
        inference_instance_id: Option<Uuid>,
        deployment_config: serde_json::Value,
        backend_name: &str,
        handle: &AdapterHandle,
    ) -> AppResult<Model> {
        let mut tx = db.begin().await?;

        let updated = sqlx::query_as::<_, Model>(
            r#"
            UPDATE models
            SET deployment_status = $3,
                inference_instance_id = $4,
                deployment_config = $5,
                updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(model_id)
        .bind(tenant_id)
        .bind(DeploymentStatus::Active.to_string())
        .bind(inference_instance_id)
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
                "backend": backend_name,
                "instance_id": inference_instance_id.map(|id| id.to_string()),
            }),
        )
        .await?;

        tx.commit().await?;
        tracing::info!(
            model_id = %model_id,
            adapter_ref = %handle.adapter_ref,
            backend = backend_name,
            instance_id = ?inference_instance_id,
            "Model deployed"
        );
        Ok(updated)
    }

    async fn reset_model_deployment(db: &PgPool, tenant_id: Uuid, model_id: Uuid) -> AppResult<()> {
        sqlx::query(
            r#"
            UPDATE models
            SET deployment_status = 'undeployed',
                inference_instance_id = NULL,
                deployment_config = '{}'::jsonb,
                updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            "#,
        )
        .bind(model_id)
        .bind(tenant_id)
        .execute(db)
        .await?;

        Ok(())
    }
}

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

    #[test]
    fn valid_deployment_statuses_parse() {
        for status in ["undeployed", "deploying", "active", "inactive"] {
            assert!(DeploymentStatus::from_str(status).is_ok());
        }
    }

    #[test]
    fn invalid_deployment_status_rejected() {
        assert!(DeploymentStatus::from_str("running").is_err());
        assert!(DeploymentStatus::from_str("").is_err());
        assert!(DeploymentStatus::from_str("ACTIVE").is_err());
    }

    #[test]
    fn adapter_name_contains_model_id() {
        let model_id = uuid::Uuid::new_v4();
        let adapter_name = format!("adapter-{model_id}");
        assert!(adapter_name.starts_with("adapter-"));
        assert!(adapter_name.contains(&model_id.to_string()));
    }

    #[test]
    fn some_adapter_path_passes_validation() {
        let adapter_path = Some("adapters/tenant/model/adapter.safetensors".to_string());
        let result = adapter_path.clone().ok_or(AppError::BadRequest {
            message: "Model has no adapter".to_string(),
        });
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "adapters/tenant/model/adapter.safetensors");
    }
}
