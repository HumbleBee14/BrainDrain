use uuid::Uuid;

use platform_shared::enums::{DeploymentStatus, TrainingJobStatus};
use platform_shared::s3_paths;
use platform_storage::ObjectStorage;

use crate::app_state::AppState;
use crate::error::{AppError, AppResult};
use crate::services::deployment_service::DeploymentService;
use crate::services::training_job_service::TrainingJobService;

/// What a purge actually removed, so callers can log and audit real numbers
/// instead of assuming the delete was a no-op.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PurgeSummary {
    pub jobs_stopped: usize,
    pub models_undeployed: usize,
    pub objects_deleted: usize,
}

/// Removes a resource and everything it owns.
pub struct PurgeService;

impl PurgeService {
    /// Delete a project and every artifact it produced.
    ///
    /// Order is stop → unload → storage → rows. Any failure aborts before the
    /// rows go, so the project stays listed and the delete is retryable.
    pub async fn purge_project(
        state: &AppState,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> AppResult<PurgeSummary> {
        let jobs = Self::project_training_jobs(state, tenant_id, project_id).await?;
        let models = Self::project_models(state, tenant_id, project_id).await?;

        let mut summary = PurgeSummary::default();

        for job in &jobs {
            if Self::stop_if_in_flight(state, tenant_id, job).await? {
                summary.jobs_stopped += 1;
            }
        }

        for model in &models {
            match model
                .deployment_status
                .parse()
                .unwrap_or(DeploymentStatus::Undeployed)
            {
                DeploymentStatus::Active => {
                    if Self::unload_active(state, tenant_id, model).await? {
                        summary.models_undeployed += 1;
                    }
                }
                // A deploy in flight would reload the adapter behind us.
                DeploymentStatus::Deploying => {
                    return Err(AppError::BadRequest {
                        message: format!(
                            "Model '{}' is mid-deployment. Wait for it to finish, then delete.",
                            model.name
                        ),
                    });
                }
                DeploymentStatus::Undeployed | DeploymentStatus::Inactive => {}
            }
        }

        let model_ids: Vec<Uuid> = models.iter().map(|m| m.id).collect();
        let job_ids: Vec<Uuid> = jobs.iter().map(|j| j.id).collect();
        summary.objects_deleted = Self::purge_project_objects(
            state.storage(),
            tenant_id,
            project_id,
            &model_ids,
            &job_ids,
        )
        .await?;

        let deleted = state.project_repo().delete(tenant_id, project_id).await?;
        if !deleted {
            return Err(AppError::NotFound {
                message: "Project not found".to_string(),
            });
        }

        tracing::info!(
            project_id = %project_id,
            tenant_id = %tenant_id,
            jobs_stopped = summary.jobs_stopped,
            models_undeployed = summary.models_undeployed,
            objects_deleted = summary.objects_deleted,
            "Project purged"
        );

        Ok(summary)
    }

    /// Delete a model: adapter, exports, the run's checkpoints, and the run
    /// itself (which cascades the model row). The run goes too, or it would
    /// keep holding a plan slot it can never use again.
    pub async fn purge_model(
        state: &AppState,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> AppResult<PurgeSummary> {
        let model = state
            .model_repo()
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        let mut summary = PurgeSummary::default();

        // Never drop a run's rows while a GPU is still attached to it.
        if let Some(job) = state
            .training_job_repo()
            .get_by_id(tenant_id, model.training_job_id)
            .await?
            && Self::stop_if_in_flight(state, tenant_id, &job).await?
        {
            summary.jobs_stopped = 1;
        }

        match model
            .deployment_status
            .parse()
            .unwrap_or(DeploymentStatus::Undeployed)
        {
            DeploymentStatus::Active => {
                if Self::unload_active(state, tenant_id, &model).await? {
                    summary.models_undeployed = 1;
                }
            }
            DeploymentStatus::Deploying => {
                return Err(AppError::BadRequest {
                    message: "This model is mid-deployment. Wait for it to finish, then delete."
                        .to_string(),
                });
            }
            DeploymentStatus::Undeployed | DeploymentStatus::Inactive => {}
        }

        let mut prefixes = s3_paths::training_job_prefixes(tenant_id, model.training_job_id);
        prefixes.push(s3_paths::export_prefix(tenant_id, model_id));
        // The row is authoritative if an older run wrote elsewhere.
        if let Some(adapter_path) = model.adapter_path.as_deref()
            && !prefixes.iter().any(|p| p == adapter_path)
        {
            prefixes.push(adapter_path.to_string());
        }
        for prefix in &prefixes {
            summary.objects_deleted += Self::erase_prefix(state.storage(), prefix).await?;
        }

        let deleted = state
            .model_repo()
            .delete_with_training_job(tenant_id, model.training_job_id)
            .await?;
        if !deleted {
            return Err(AppError::NotFound {
                message: "Training run for this model not found".to_string(),
            });
        }

        tracing::info!(
            model_id = %model_id,
            training_job_id = %model.training_job_id,
            tenant_id = %tenant_id,
            objects_deleted = summary.objects_deleted,
            "Model purged"
        );

        Ok(summary)
    }

    /// Delete one prefix. The backend error is logged, not returned — the
    /// client only needs to know nothing was deleted and a retry is safe.
    async fn erase_prefix(storage: &impl ObjectStorage, prefix: &str) -> AppResult<usize> {
        storage.delete_prefix(prefix).await.map_err(|e| {
            tracing::error!(prefix = prefix, error = %e, "Purge aborted: prefix delete failed");
            AppError::ServiceUnavailable {
                message: "Could not erase the stored files, so nothing was deleted. Try again."
                    .to_string(),
            }
        })
    }

    /// Take a deployed model off its serving engine. A model whose instance is
    /// already gone has nothing to unload, and failing there would leave it
    /// undeletable forever. Returns whether an adapter was unloaded.
    async fn unload_active(
        state: &AppState,
        tenant_id: Uuid,
        model: &platform_db::models::Model,
    ) -> AppResult<bool> {
        if let Some(instance_id) = model.inference_instance_id
            && state
                .inference_instance_repo()
                .get_by_id(instance_id)
                .await?
                .is_none()
        {
            tracing::warn!(
                model_id = %model.id,
                instance_id = %instance_id,
                "Model is marked active but its serving instance is gone; deleting without unload"
            );
            return Ok(false);
        }

        DeploymentService::undeploy(state, tenant_id, model.id).await?;
        Ok(true)
    }

    /// Cancel a queued or running job so the GPU stops and the partial run is
    /// billed before its rows go. Returns whether anything was stopped.
    async fn stop_if_in_flight(
        state: &AppState,
        tenant_id: Uuid,
        job: &platform_db::models::TrainingJob,
    ) -> AppResult<bool> {
        let status: TrainingJobStatus = job.status.parse().unwrap_or(TrainingJobStatus::Failed);
        if !matches!(
            status,
            TrainingJobStatus::Pending
                | TrainingJobStatus::CostApproval
                | TrainingJobStatus::Provisioning
                | TrainingJobStatus::Training
        ) {
            return Ok(false);
        }

        let cancelled = TrainingJobService::cancel(
            state.training_job_repo(),
            state.tenant_repo(),
            state.orchestrator(),
            tenant_id,
            job.id,
        )
        .await;

        match cancelled {
            Ok(_) => Ok(true),
            Err(e) => {
                // The run may have settled between the read and this cancel —
                // the state we wanted. Re-read rather than parse the error.
                let settled = state
                    .training_job_repo()
                    .get_by_id(tenant_id, job.id)
                    .await?
                    .map(|j| {
                        matches!(
                            j.status.parse().unwrap_or(TrainingJobStatus::Failed),
                            TrainingJobStatus::Completed
                                | TrainingJobStatus::Failed
                                | TrainingJobStatus::Cancelled
                        )
                    })
                    .unwrap_or(true);

                if settled {
                    tracing::info!(
                        training_job_id = %job.id,
                        "Run settled on its own while the purge was cancelling it"
                    );
                    Ok(false)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Project-keyed prefixes, each run's adapter and checkpoints (keyed by job
    /// id), and each model's exports (keyed by model id).
    pub async fn purge_project_objects(
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
        project_id: Uuid,
        model_ids: &[Uuid],
        job_ids: &[Uuid],
    ) -> AppResult<usize> {
        let mut prefixes = s3_paths::project_prefixes(tenant_id, project_id);
        for model_id in model_ids {
            prefixes.push(s3_paths::export_prefix(tenant_id, *model_id));
        }
        for job_id in job_ids {
            prefixes.extend(s3_paths::training_job_prefixes(tenant_id, *job_id));
        }

        let mut objects_deleted = 0;
        for prefix in &prefixes {
            objects_deleted += Self::erase_prefix(storage, prefix).await?;
        }
        Ok(objects_deleted)
    }

    async fn project_models(
        state: &AppState,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> AppResult<Vec<platform_db::models::Model>> {
        let count = state
            .model_repo()
            .count_by_project(tenant_id, project_id)
            .await?;
        if count == 0 {
            return Ok(Vec::new());
        }
        state
            .model_repo()
            .list_by_project(tenant_id, project_id, 0, count)
            .await
    }

    async fn project_training_jobs(
        state: &AppState,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> AppResult<Vec<platform_db::models::TrainingJob>> {
        let count = state
            .training_job_repo()
            .count_by_project(tenant_id, project_id)
            .await?;
        if count == 0 {
            return Ok(Vec::new());
        }
        state
            .training_job_repo()
            .list_by_project(tenant_id, project_id, 0, count)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform_storage::memory::InMemoryStorage;

    fn ids() -> (Uuid, Uuid, Uuid, Uuid) {
        (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
    }

    /// Written where the trainer writes it: under the job id, which is what
    /// lands in `models.adapter_path`.
    fn trained_adapter_key(tenant: Uuid, job: Uuid) -> String {
        format!(
            "{}adapter_model.safetensors",
            s3_paths::adapter_prefix(tenant, job)
        )
    }

    #[tokio::test]
    async fn purges_project_model_and_checkpoint_objects() {
        let (tenant, project, model, job) = ids();
        let storage = InMemoryStorage::new();

        let keys = [
            s3_paths::upload_path(tenant, project, Uuid::new_v4(), "pdf"),
            s3_paths::parsed_path(tenant, project, Uuid::new_v4()),
            s3_paths::dataset_path(tenant, project, Uuid::new_v4()),
            trained_adapter_key(tenant, job),
            s3_paths::export_path(tenant, model, "model.gguf"),
            format!("{}shard-0.pt", s3_paths::checkpoint_prefix(tenant, job)),
            format!("chunks/{tenant}/{project}/batch-0.jsonl"),
            // Teacher logprob artifacts nest under the dataset's own key.
            format!("datasets/{tenant}/{project}/{model}-teacher-logprobs/abc/0001.json"),
        ];
        for key in &keys {
            storage
                .put(
                    key,
                    bytes::Bytes::from_static(b"x"),
                    "application/octet-stream",
                )
                .await
                .unwrap();
        }

        let deleted =
            PurgeService::purge_project_objects(&storage, tenant, project, &[model], &[job])
                .await
                .unwrap();

        assert_eq!(deleted, keys.len());
        for key in &keys {
            assert!(
                !storage.exists(key).await.unwrap(),
                "{key} survived the purge"
            );
        }
    }

    #[tokio::test]
    async fn leaves_other_projects_and_models_untouched() {
        let (tenant, project, model, job) = ids();
        let other_project = Uuid::new_v4();
        let other_model = Uuid::new_v4();
        let storage = InMemoryStorage::new();

        let survivors = [
            s3_paths::dataset_path(tenant, other_project, Uuid::new_v4()),
            trained_adapter_key(tenant, Uuid::new_v4()),
            s3_paths::export_path(tenant, other_model, "model.gguf"),
        ];
        for key in &survivors {
            storage
                .put(
                    key,
                    bytes::Bytes::from_static(b"x"),
                    "application/octet-stream",
                )
                .await
                .unwrap();
        }

        let deleted =
            PurgeService::purge_project_objects(&storage, tenant, project, &[model], &[job])
                .await
                .unwrap();

        assert_eq!(deleted, 0);
        for key in &survivors {
            assert!(
                storage.exists(key).await.unwrap(),
                "{key} was wrongly deleted"
            );
        }
    }
}
