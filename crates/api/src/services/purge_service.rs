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

/// Removes a resource and everything it owns — running work, served adapters,
/// stored objects, and finally the rows.
pub struct PurgeService;

impl PurgeService {
    /// Delete a project and every artifact it produced.
    ///
    /// Order is stop → unload → storage → rows, and every step propagates its
    /// error instead of warning past it. Aborting leaves the project row in
    /// place, so the caller simply deletes again: stopping an already-stopped
    /// job and deleting an already-gone object are both no-ops, and the retry
    /// carries on from where it failed. Deleting the rows first (the previous
    /// behaviour) stranded every adapter, dataset and export in object storage
    /// with nothing left pointing at them.
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
                    DeploymentService::undeploy(state, tenant_id, model.id).await?;
                    summary.models_undeployed += 1;
                }
                // A half-finished deploy would put the adapter back after we
                // unloaded it, so refuse rather than race it.
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

    /// Delete one model: its adapter, its exports, the checkpoints of the run
    /// that produced it, and the run itself (which cascades the model row).
    ///
    /// Same ordering guarantee as [`Self::purge_project`] — unload, erase, then
    /// rows — so a failure leaves the model listed and the delete retryable.
    /// The run goes with the model because they are one unit: a completed run
    /// whose model no longer exists still holds a plan slot and can never
    /// produce another.
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

        // A model normally implies a finished run, but never delete a run's rows
        // while a GPU is still attached to it.
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
                DeploymentService::undeploy(state, tenant_id, model_id).await?;
                summary.models_undeployed = 1;
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
        // The row records where the adapter actually landed; trust it over the
        // reconstructed prefix in case an older run wrote somewhere else.
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

    /// Delete one prefix, turning a storage failure into an answer the caller
    /// can act on. The underlying error is logged, never returned: the client
    /// only needs to know that nothing was deleted and a retry is safe.
    async fn erase_prefix(storage: &impl ObjectStorage, prefix: &str) -> AppResult<usize> {
        storage.delete_prefix(prefix).await.map_err(|e| {
            tracing::error!(prefix = prefix, error = %e, "Purge aborted: prefix delete failed");
            AppError::ServiceUnavailable {
                message: "Could not erase the stored files, so nothing was deleted. Try again."
                    .to_string(),
            }
        })
    }

    /// Cancel a run that is still queued or on a GPU, so the workflow stops and
    /// the partial run is billed before its rows are deleted. Returns whether
    /// anything was stopped; terminal runs are left alone.
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

        TrainingJobService::cancel(
            state.training_job_repo(),
            state.tenant_repo(),
            state.orchestrator(),
            tenant_id,
            job.id,
        )
        .await?;
        Ok(true)
    }

    /// Every stored object belonging to a project: the project-keyed prefixes,
    /// the adapter and checkpoints of each run (keyed by job id), and the
    /// exports of each model (keyed by model id).
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

    /// Written exactly where the trainer writes it: under the TRAINING JOB id,
    /// which is what lands in `models.adapter_path`. Keying this by model id
    /// instead would let every adapter survive a project delete.
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
            // Teacher logprob artifacts live under the dataset's own key, so the
            // dataset prefix must reclaim them too.
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
