use platform_db::models::InferenceInstance;
use platform_shared::enums::{InferenceInstanceHealthStatus, InferenceInstanceLifecycleState};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dto::inference_instance::{
    CreateInferenceInstanceRequest, InferenceInstanceResponse,
    UpdateInferenceInstanceLifecycleRequest,
};
use crate::error::{AppError, AppResult};
pub struct InferenceInstanceService;

impl InferenceInstanceService {
    pub async fn register(
        state: &AppState,
        request: CreateInferenceInstanceRequest,
    ) -> AppResult<InferenceInstanceResponse> {
        let backend =
            state.build_inference_backend_for_instance(&request.backend_type, &request.base_url);
        let initial_health =
            if Self::probe_instance(&state.http_client().clone(), backend.base_url()).await {
                InferenceInstanceHealthStatus::Healthy
            } else {
                InferenceInstanceHealthStatus::Unhealthy
            };

        let instance = state
            .inference_instance_repo()
            .create(
                &request.name,
                &request.base_url,
                &request.backend_type,
                request.gpu_class.as_deref(),
                &request.base_model,
                request.max_adapters,
                initial_health,
                InferenceInstanceLifecycleState::Ready,
                request.metadata,
            )
            .await?;

        Ok(instance.into())
    }

    pub async fn list(state: &AppState) -> AppResult<Vec<InferenceInstanceResponse>> {
        let instances = state.inference_instance_repo().list().await?;
        Ok(instances.into_iter().map(Into::into).collect())
    }

    pub async fn update_lifecycle(
        state: &AppState,
        id: Uuid,
        request: UpdateInferenceInstanceLifecycleRequest,
    ) -> AppResult<InferenceInstanceResponse> {
        // For retirement, use a conditional UPDATE to avoid TOCTOU race:
        // a slot could be claimed between a get_by_id check and the update.
        if matches!(
            request.lifecycle_state,
            InferenceInstanceLifecycleState::Retired
        ) {
            let updated = state
                .inference_instance_repo()
                .retire_if_empty(id)
                .await?
                .ok_or(AppError::Conflict {
                    message: "Cannot retire: instance not found or has active adapters".to_string(),
                })?;
            return Ok(updated.into());
        }

        let updated = state
            .inference_instance_repo()
            .update_lifecycle_state(id, request.lifecycle_state)
            .await?
            .ok_or(AppError::NotFound {
                message: "Inference instance not found".to_string(),
            })?;

        Ok(updated.into())
    }

    pub async fn delete(state: &AppState, id: Uuid) -> AppResult<()> {
        // Conditional DELETE to avoid TOCTOU race: only delete if
        // active_adapter_count is 0 at the moment of deletion.
        let deleted = state.inference_instance_repo().delete_if_empty(id).await?;

        if !deleted {
            return Err(AppError::Conflict {
                message: "Cannot delete: instance not found or has active adapters".to_string(),
            });
        }

        Ok(())
    }

    pub async fn run_health_probes(state: &AppState) -> AppResult<()> {
        let instances = state
            .inference_instance_repo()
            .list_for_healthcheck()
            .await?;
        for instance in instances {
            let healthy =
                Self::probe_instance(&state.http_client().clone(), &instance.base_url).await;
            let health = if healthy {
                InferenceInstanceHealthStatus::Healthy
            } else {
                InferenceInstanceHealthStatus::Unhealthy
            };
            if let Err(e) = state
                .inference_instance_repo()
                .update_health(instance.id, health)
                .await
            {
                tracing::warn!(instance_id = %instance.id, error = %e, "Failed to update inference instance health");
            }
        }

        Ok(())
    }

    pub async fn get_routable_instance(
        state: &AppState,
        instance_id: Uuid,
    ) -> AppResult<InferenceInstance> {
        let instance = state
            .inference_instance_repo()
            .get_by_id(instance_id)
            .await?
            .ok_or(AppError::ServiceUnavailable {
                message: "Assigned inference instance no longer exists".to_string(),
            })?;

        if instance.health_status != InferenceInstanceHealthStatus::Healthy.to_string() {
            return Err(AppError::ServiceUnavailable {
                message: format!("Inference instance '{}' is not healthy", instance.name),
            });
        }

        if instance.lifecycle_state == InferenceInstanceLifecycleState::Retired.to_string() {
            return Err(AppError::ServiceUnavailable {
                message: format!("Inference instance '{}' is retired", instance.name),
            });
        }

        Ok(instance)
    }

    async fn probe_instance(http_client: &reqwest::Client, base_url: &str) -> bool {
        let url = format!("{}/health", base_url.trim_end_matches('/'));
        match http_client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(e) => {
                tracing::warn!(base_url, error = %e, "Inference instance health probe failed");
                false
            }
        }
    }
}
