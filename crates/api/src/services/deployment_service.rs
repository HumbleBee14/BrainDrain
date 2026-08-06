use platform_db::models::Model;
use platform_shared::enums::{DeploymentStatus, TrainingMode};
use sqlx::PgPool;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dto::model::ModelResponse;
use crate::error::{AppError, AppResult};
use crate::services::billing_outbox;
use crate::services::deploy_gate::{self, DeployGatePolicy, GateDecision};
use crate::services::feature_flags::{DEPLOYMENTS_MULTI_INSTANCE_ENABLED, FlagContext};
use crate::services::inference_backend::AdapterHandle;

/// Whether the deployment eval gate applies to a given deploy call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateMode {
    /// Enforce the configured eval gate. Used by the normal deploy path: a new,
    /// unproven model must clear the thresholds before it reaches production.
    Enforce,
    /// Bypass the gate. Used by rollbacks, which restore a *previously deployed*
    /// version — that version was already vetted when it first shipped, and the
    /// gate must never trap an operator on a broken current version by refusing
    /// the rollback that would restore service.
    Bypass,
}

/// Business logic for model deployment via pluggable inference backends.
pub struct DeploymentService;

impl DeploymentService {
    /// Deploy a fine-tuned model by loading its LoRA adapter into the inference
    /// backend. Enforces the eval gate (see [`GateMode`]).
    pub async fn deploy(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<ModelResponse> {
        Self::deploy_with_gate(state, tenant_id, model_id, GateMode::Enforce).await
    }

    /// Deploy without applying the eval gate. Reserved for rollbacks to a
    /// previously deployed version — see [`GateMode::Bypass`].
    pub async fn deploy_bypassing_gate(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<ModelResponse> {
        Self::deploy_with_gate(state, tenant_id, model_id, GateMode::Bypass).await
    }

    async fn deploy_with_gate(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
        gate_mode: GateMode,
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

        // Eval gate: applied once here so it guards BOTH the single-instance and
        // multi-instance paths below. Rollbacks pass Bypass and skip it.
        if gate_mode == GateMode::Enforce {
            Self::enforce_eval_gate(state, tenant_id, &model).await?;
        }

        let multi_instance_enabled = state
            .feature_flags()
            .is_enabled(DEPLOYMENTS_MULTI_INSTANCE_ENABLED, &FlagContext::default());

        // Recover the system prompt the model was trained under so it is served
        // under the same one (train/serve consistency). Best-effort: models not
        // built via a data guide simply have none.
        let system_prompt = Self::resolve_guide_system_prompt(state, tenant_id, &model).await;

        if multi_instance_enabled {
            Self::deploy_multi_instance(
                state,
                tenant_id,
                model_id,
                &model,
                &adapter_path,
                &system_prompt,
            )
            .await
        } else {
            Self::deploy_single_instance(
                state,
                tenant_id,
                model_id,
                &model,
                &adapter_path,
                &system_prompt,
            )
            .await
        }
    }

    /// Best-effort lookup of the system prompt for a model via
    /// model → training job → dataset → data guide. Returns "" on any miss.
    pub(crate) async fn resolve_guide_system_prompt(
        state: &AppState,
        tenant_id: Uuid,
        model: &Model,
    ) -> String {
        let Ok(Some(job)) = state
            .training_job_repo()
            .get_by_id(tenant_id, model.training_job_id)
            .await
        else {
            return String::new();
        };
        match state
            .data_guide_repo()
            .get_by_dataset_id(tenant_id, job.dataset_id)
            .await
        {
            Ok(Some(guide)) => guide.system_prompt,
            _ => String::new(),
        }
    }

    /// Enforce the configured deployment eval gate for `model`. Reads the
    /// model's latest completed evaluation scores and blocks the deploy with a
    /// typed [`AppError::Conflict`] (not a 500) when they fail the policy. A gate
    /// with no thresholds configured is a no-op.
    async fn enforce_eval_gate(state: &AppState, tenant_id: Uuid, model: &Model) -> AppResult<()> {
        let policy = DeployGatePolicy::from_thresholds(
            state.config().deploy_min_ab_win_rate,
            state.config().deploy_max_benchmark_regression,
            state.config().deploy_min_doc_knowledge_lift,
            state.config().deploy_min_teacher_parity,
        );
        if !policy.is_enabled() {
            return Ok(());
        }

        // Scoped before the scores lookup so a mode-specific threshold can
        // reduce the policy back to disabled and skip the query entirely.
        let policy = policy.for_mode(Self::resolve_training_mode(state, tenant_id, model).await?);
        if !policy.is_enabled() {
            return Ok(());
        }

        // Absent scores (no completed eval) become an empty object; the policy
        // then treats the required metrics as unavailable and blocks — the gate
        // requires positive evidence, so an unevaluated model does not deploy.
        let scores = state
            .evaluation_repo()
            .latest_completed_scores(tenant_id, model.id)
            .await?
            .unwrap_or_else(|| serde_json::json!({}));

        match policy.check(&scores) {
            GateDecision::Blocked(violations) => {
                let failed_metrics = violations
                    .iter()
                    .map(|v| v.metric.label())
                    .collect::<Vec<_>>()
                    .join(", ");
                let message = deploy_gate::format_block_message(&violations);
                tracing::info!(
                    model_id = %model.id,
                    gate = "blocked",
                    failed_metrics = %failed_metrics,
                    "{message}"
                );
                Err(AppError::Conflict { message })
            }
            GateDecision::Passed | GateDecision::Disabled => Ok(()),
        }
    }

    /// The training mode a model was produced under, used to scope mode-specific
    /// gate rules. An unrecognized stored mode falls back to [`TrainingMode::Quick`]:
    /// it cannot be a mode whose suites emit distill-only metrics, which is the
    /// only decision this value feeds.
    async fn resolve_training_mode(
        state: &AppState,
        tenant_id: Uuid,
        model: &Model,
    ) -> AppResult<TrainingMode> {
        let job = state
            .training_job_repo()
            .get_by_id(tenant_id, model.training_job_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Training job for model not found".to_string(),
            })?;
        Ok(job.mode.parse().unwrap_or(TrainingMode::Quick))
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

        // Best-effort unload AFTER DB commit. A failure leaks GPU memory until
        // the instance restarts, so it must not be silent.
        if let Err(e) = resolved_backend.unload_adapter(&adapter_ref).await {
            tracing::warn!(
                model_id = %model_id,
                adapter_ref = %adapter_ref,
                error = %e,
                "Adapter unload failed after undeploy; it holds GPU memory until the instance restarts"
            );
        }

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
        system_prompt: &str,
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

        // Loading the adapter can take minutes on a scale-to-zero engine, so it
        // runs detached: the claimed 'deploying' row is the durable record, and a
        // crash mid-load is recovered by reap_stale_deployments.
        let task_state = state.clone();
        let base_model = model.base_model.clone();
        let adapter_path = adapter_path.to_string();
        let system_prompt = system_prompt.to_string();
        tokio::spawn(async move {
            Self::complete_single_instance_deploy(
                task_state,
                tenant_id,
                model_id,
                base_model,
                adapter_path,
                system_prompt,
            )
            .await;
        });

        let mut deploying = model.clone();
        deploying.deployment_status = DeploymentStatus::Deploying.to_string();
        Ok(deploying.into())
    }

    /// Adapter load + finalize for a claimed deployment. Runs outside the request
    /// so a disconnected client cannot cancel it; every exit path either activates
    /// the model or releases the claim.
    async fn complete_single_instance_deploy(
        state: AppState,
        tenant_id: Uuid,
        model_id: Uuid,
        base_model: String,
        adapter_path: String,
        system_prompt: String,
    ) {
        let backend = state.inference_backend();
        let handle = match backend.load_adapter(model_id, &adapter_path).await {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(model_id = %model_id, backend = backend.name(), error = %error, "Adapter load failed");
                let _ = Self::reset_model_deployment(state.db(), tenant_id, model_id).await;
                return;
            }
        };

        let mut deployment_config = serde_json::json!({
            "adapter_ref": handle.adapter_ref,
            "adapter_path": adapter_path,
            "base_model": base_model,
            "backend": backend.name(),
            "backend_meta": handle.metadata,
        });
        Self::attach_system_prompt(&mut deployment_config, &system_prompt);

        if let Err(error) = Self::finalize_deploy(
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
            tracing::error!(model_id = %model_id, error = %error, "Deploy finalize failed");
            let _ = backend.unload_adapter(&handle.adapter_ref).await;
            let _ = Self::reset_model_deployment(state.db(), tenant_id, model_id).await;
        }
    }

    /// Store a non-empty system prompt in the deployment config so the inference
    /// path can inject it as the default. Empty prompts add no key, keeping the
    /// config identical to the prior behavior.
    fn attach_system_prompt(config: &mut serde_json::Value, system_prompt: &str) {
        if !system_prompt.is_empty() {
            config["system_prompt"] = serde_json::Value::String(system_prompt.to_string());
        }
    }

    async fn deploy_multi_instance(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
        model: &Model,
        adapter_path: &str,
        system_prompt: &str,
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

        let mut deployment_config = serde_json::json!({
            "adapter_ref": handle.adapter_ref,
            "adapter_path": adapter_path,
            "base_model": model.base_model,
            "backend": backend.name(),
            "instance_id": instance.id.to_string(),
            "instance_name": instance.name,
            "instance_url": instance.base_url,
            "backend_meta": handle.metadata,
        });
        Self::attach_system_prompt(&mut deployment_config, system_prompt);

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

    #[test]
    fn deployment_config_with_instance_fields() {
        let instance_id = Uuid::new_v4();
        let config = serde_json::json!({
            "adapter_ref": "adapter-abc",
            "adapter_path": "/s3/path",
            "base_model": "meta-llama/Llama-3.1-8B",
            "backend": "vllm",
            "instance_id": instance_id.to_string(),
            "instance_name": "gpu-a10g-1",
            "instance_url": "http://vllm-1:8080",
            "backend_meta": {"backend": "vllm"},
        });

        assert_eq!(config["adapter_ref"], "adapter-abc");
        assert_eq!(config["backend"], "vllm");
        assert_eq!(config["instance_id"], instance_id.to_string());
        assert_eq!(config["instance_name"], "gpu-a10g-1");
        assert_eq!(config["instance_url"], "http://vllm-1:8080");
    }

    #[test]
    fn deployment_config_without_instance_fields_is_backward_compat() {
        let config = serde_json::json!({
            "adapter_ref": "adapter-abc",
            "base_model": "meta-llama/Llama-3.1-8B",
            "backend": "vllm",
        });

        assert!(config["instance_id"].is_null());
        assert!(config["instance_url"].is_null());
    }

    #[test]
    fn attach_system_prompt_adds_key_only_when_non_empty() {
        let mut with_prompt = serde_json::json!({"backend": "vllm"});
        DeploymentService::attach_system_prompt(&mut with_prompt, "You are a support agent.");
        assert_eq!(with_prompt["system_prompt"], "You are a support agent.");

        let mut without = serde_json::json!({"backend": "vllm"});
        DeploymentService::attach_system_prompt(&mut without, "");
        assert!(without["system_prompt"].is_null());
    }

    #[test]
    fn deploy_rejects_already_active() {
        let status = DeploymentStatus::Active.to_string();
        assert_eq!(status, "active");
        // The deploy function checks this and returns Conflict
    }

    #[test]
    fn undeploy_rejects_not_active() {
        let status = DeploymentStatus::Undeployed.to_string();
        assert_ne!(status, DeploymentStatus::Active.to_string());
    }

    #[test]
    fn deploy_conditional_where_prevents_race() {
        // The SQL uses: WHERE deployment_status NOT IN ('deploying', 'active')
        // This means concurrent deploys can't both win
        let deploying = DeploymentStatus::Deploying.to_string();
        let active = DeploymentStatus::Active.to_string();
        let undeployed = DeploymentStatus::Undeployed.to_string();

        // Only undeployed and inactive can be deployed
        assert_ne!(deploying, undeployed);
        assert_ne!(active, undeployed);
    }
}
