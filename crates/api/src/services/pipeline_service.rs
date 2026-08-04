use platform_shared::enums::{
    DatasetStatus, DeploymentStatus, DocumentStatus, EvaluationStatus, TrainingJobStatus,
};
use serde_json::json;
use uuid::Uuid;

use crate::dto::pipeline::{
    DatasetStatusCounts, DocumentStatusCounts, EvaluationStatusCounts, ModelStatusCounts,
    ProjectPipelineStatus, TrainingJobStatusCounts, TriggerFullPipelineResponse,
    TriggerParseResponse, TriggerRefineResponse,
};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{
    DatasetRepository, DocumentRepository, EvaluationRepository, ModelRepository,
    TrainingJobRepository,
};
use crate::services::secret_cipher::SecretCipher;
use crate::services::teacher::config::{
    TeacherConfigDto, ValidatedTeacher, check_distill_teacher_contract, validate_teacher_for_launch,
};
use crate::temporal::{TraceContext, WorkflowOrchestrator};

/// Coerce a request `config` JSON into an object so keys can be injected
/// (an omitted config deserializes to `null`).
fn as_config_map(
    config: &mut serde_json::Value,
) -> &mut serde_json::Map<String, serde_json::Value> {
    if !config.is_object() {
        *config = serde_json::Value::Object(serde_json::Map::new());
    }
    match config {
        serde_json::Value::Object(map) => map,
        _ => unreachable!("config was just coerced to an object"),
    }
}

async fn validate_optional_teacher(
    teacher: Option<TeacherConfigDto>,
    cipher: &SecretCipher,
) -> AppResult<Option<ValidatedTeacher>> {
    match teacher {
        Some(dto) => Ok(Some(validate_teacher_for_launch(&dto, cipher).await?)),
        None => Ok(None),
    }
}

/// Business logic for pipeline orchestration.
///
/// Validates preconditions and triggers Temporal workflows.
pub struct PipelineService;

impl PipelineService {
    /// Trigger document parsing for all unparsed documents in a project.
    ///
    /// Finds documents with status "uploaded" and starts an IngestWorkflow.
    pub async fn trigger_parse(
        doc_repo: &dyn DocumentRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        project_id: Uuid,
        trace_ctx: TraceContext,
    ) -> AppResult<TriggerParseResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Pipeline workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        let docs = doc_repo
            .list_by_status(tenant_id, project_id, DocumentStatus::Uploaded)
            .await?;

        if docs.is_empty() {
            return Err(AppError::BadRequest {
                message: "No documents ready for parsing (status must be 'uploaded')".to_string(),
            });
        }

        let doc_ids: Vec<Uuid> = docs.iter().map(|d| d.id).collect();
        let doc_count = doc_ids.len();

        let result = orchestrator
            .start_ingest(tenant_id, project_id, doc_ids, trace_ctx)
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start IngestWorkflow: {e}"))
            })?;

        tracing::info!(
            project_id = %project_id,
            workflow_id = %result.workflow_id,
            document_count = doc_count,
            "IngestWorkflow started"
        );

        Ok(TriggerParseResponse {
            workflow_id: result.workflow_id,
            document_count: doc_count,
        })
    }

    /// Trigger data refinement for parsed documents in a project.
    ///
    /// Finds documents with status "parsed" and starts a RefineWorkflow.
    #[allow(clippy::too_many_arguments)]
    pub async fn trigger_refine(
        doc_repo: &dyn DocumentRepository,
        dataset_repo: &dyn DatasetRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        cipher: &SecretCipher,
        tenant_id: Uuid,
        project_id: Uuid,
        task_type: &str,
        mut config: serde_json::Value,
        teacher: Option<TeacherConfigDto>,
        stale_minutes: i64,
        trace_ctx: TraceContext,
    ) -> AppResult<TriggerRefineResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Pipeline workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        let teacher = validate_optional_teacher(teacher, cipher).await?;

        let docs = doc_repo
            .list_by_status(tenant_id, project_id, DocumentStatus::Parsed)
            .await?;

        if docs.is_empty() {
            return Err(AppError::BadRequest {
                message: "No parsed documents available for refinement".to_string(),
            });
        }

        let doc_ids: Vec<Uuid> = docs.iter().map(|d| d.id).collect();
        let doc_count = doc_ids.len();

        // A killed worker never reaches mark_failed, so clear those rows here
        // rather than leaving them generating forever.
        if let Err(e) = dataset_repo.reap_stale_generating(stale_minutes).await {
            tracing::warn!(error = %e, "Failed to reap stale generating datasets");
        }

        // Reserve the row before starting the run: generation takes minutes of
        // LLM calls, and without a durable row there is nothing for the UI to
        // show meanwhile and nowhere to record a failure.
        //
        // The persisted dataset config gets the credential-free teacher
        // provenance block; the full teacher block (encrypted key included)
        // rides only in the workflow input.
        let dataset_id = Uuid::new_v4();
        let mut persisted_config = config.clone();
        if let Some(t) = &teacher {
            as_config_map(&mut persisted_config).insert(
                "teacher".to_string(),
                t.provenance_value(&chrono::Utc::now().to_rfc3339()),
            );
        }
        dataset_repo
            .create_generating(
                tenant_id,
                project_id,
                dataset_id,
                format!("Generating from {doc_count} document(s)"),
                persisted_config,
            )
            .await?;

        let config_map = as_config_map(&mut config);
        config_map.insert("dataset_id".to_string(), json!(dataset_id.to_string()));
        if let Some(t) = &teacher {
            config_map.insert("teacher".to_string(), t.workflow_value());
        }

        let started = orchestrator
            .start_refine(tenant_id, project_id, doc_ids, task_type, config, trace_ctx)
            .await;

        let result = match started {
            Ok(result) => result,
            Err(e) => {
                let message = format!("Failed to start generation: {e}");
                if let Err(mark_err) = dataset_repo
                    .mark_failed(tenant_id, dataset_id, message.clone())
                    .await
                {
                    tracing::error!(error = %mark_err, %dataset_id, "Failed to mark dataset failed");
                }
                return Err(AppError::Internal(anyhow::anyhow!(message)));
            }
        };

        tracing::info!(
            project_id = %project_id,
            workflow_id = %result.workflow_id,
            dataset_id = %dataset_id,
            document_count = doc_count,
            task_type = task_type,
            "RefineWorkflow started"
        );

        Ok(TriggerRefineResponse {
            workflow_id: result.workflow_id,
            document_count: doc_count,
            teacher_policy: teacher.map(|t| t.policy),
        })
    }

    /// Trigger the full pipeline: ingest → refine → train → evaluate → (optional deploy).
    ///
    /// Starts a FullPipelineWorkflow for all uploaded documents in a project.
    #[allow(clippy::too_many_arguments)]
    pub async fn trigger_full_pipeline(
        doc_repo: &dyn DocumentRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        cipher: &SecretCipher,
        tenant_id: Uuid,
        project_id: Uuid,
        task_type: &str,
        base_model: &str,
        mut training_config: serde_json::Value,
        teacher: Option<TeacherConfigDto>,
        trace_ctx: TraceContext,
    ) -> AppResult<TriggerFullPipelineResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Pipeline workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        // Distill runs are teacher-generated by contract: the teacher writes
        // the training data, then the student trains on it. Enforce both
        // directions so job records and dataset provenance always agree.
        let mode = training_config
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("quick");
        check_distill_teacher_contract(mode == "distill", teacher.is_some())?;
        let teacher = validate_optional_teacher(teacher, cipher).await?;
        if let Some(t) = &teacher {
            as_config_map(&mut training_config).insert("teacher".to_string(), t.workflow_value());
        }

        // Collect all documents that haven't failed (uploaded or parsed)
        let (uploaded, parsed) = tokio::try_join!(
            doc_repo.list_by_status(tenant_id, project_id, DocumentStatus::Uploaded),
            doc_repo.list_by_status(tenant_id, project_id, DocumentStatus::Parsed),
        )?;

        let mut doc_ids: Vec<Uuid> = uploaded.iter().map(|d| d.id).collect();
        doc_ids.extend(parsed.iter().map(|d| d.id));

        if doc_ids.is_empty() {
            return Err(AppError::BadRequest {
                message:
                    "No documents available for the pipeline (need uploaded or parsed documents)"
                        .to_string(),
            });
        }

        let doc_count = doc_ids.len();

        let result = orchestrator
            .start_full_pipeline(
                tenant_id,
                project_id,
                doc_ids,
                task_type,
                base_model,
                training_config,
                trace_ctx,
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start FullPipelineWorkflow: {e}"))
            })?;

        tracing::info!(
            project_id = %project_id,
            workflow_id = %result.workflow_id,
            document_count = doc_count,
            base_model = base_model,
            "FullPipelineWorkflow started"
        );

        Ok(TriggerFullPipelineResponse {
            workflow_id: result.workflow_id,
            document_count: doc_count,
            teacher_policy: teacher.map(|t| t.policy),
        })
    }

    /// Get aggregate pipeline status for a project.
    pub async fn get_status(
        doc_repo: &dyn DocumentRepository,
        dataset_repo: &dyn DatasetRepository,
        training_repo: &dyn TrainingJobRepository,
        model_repo: &dyn ModelRepository,
        eval_repo: &dyn EvaluationRepository,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> AppResult<ProjectPipelineStatus> {
        let (
            total_docs,
            uploaded,
            parsing,
            parsed,
            doc_failed,
            total_datasets,
            generating,
            review_pending,
            approved,
            datasets_failed,
            total_jobs,
            jobs_pending,
            jobs_training,
            jobs_completed,
            jobs_failed,
            total_models,
            models_undeployed,
            models_active,
            total_evals,
            evals_running,
            evals_completed,
            evals_failed,
        ) = tokio::try_join!(
            doc_repo.count_by_project(tenant_id, project_id),
            doc_repo.count_by_status(tenant_id, project_id, DocumentStatus::Uploaded),
            doc_repo.count_by_status(tenant_id, project_id, DocumentStatus::Parsing),
            doc_repo.count_by_status(tenant_id, project_id, DocumentStatus::Parsed),
            doc_repo.count_by_status(tenant_id, project_id, DocumentStatus::Failed),
            dataset_repo.count_by_project(tenant_id, project_id),
            dataset_repo.count_by_status(tenant_id, project_id, DatasetStatus::Generating),
            dataset_repo.count_by_status(tenant_id, project_id, DatasetStatus::ReviewPending),
            dataset_repo.count_by_status(tenant_id, project_id, DatasetStatus::Approved),
            dataset_repo.count_by_status(tenant_id, project_id, DatasetStatus::Failed),
            training_repo.count_by_project(tenant_id, project_id),
            training_repo.count_by_status(tenant_id, project_id, TrainingJobStatus::Pending),
            training_repo.count_by_status(tenant_id, project_id, TrainingJobStatus::Training),
            training_repo.count_by_status(tenant_id, project_id, TrainingJobStatus::Completed),
            training_repo.count_by_status(tenant_id, project_id, TrainingJobStatus::Failed),
            model_repo.count_by_project(tenant_id, project_id),
            model_repo.count_by_deployment_status(
                tenant_id,
                project_id,
                DeploymentStatus::Undeployed
            ),
            model_repo.count_by_deployment_status(tenant_id, project_id, DeploymentStatus::Active),
            eval_repo.count_by_project(tenant_id, project_id),
            eval_repo.count_by_project_status(tenant_id, project_id, EvaluationStatus::Running),
            eval_repo.count_by_project_status(tenant_id, project_id, EvaluationStatus::Completed),
            eval_repo.count_by_project_status(tenant_id, project_id, EvaluationStatus::Failed),
        )?;

        Ok(ProjectPipelineStatus {
            project_id: project_id.to_string(),
            documents: DocumentStatusCounts {
                total: total_docs,
                uploaded,
                parsing,
                parsed,
                failed: doc_failed,
            },
            datasets: DatasetStatusCounts {
                total: total_datasets,
                generating,
                review_pending,
                approved,
                failed: datasets_failed,
            },
            training_jobs: TrainingJobStatusCounts {
                total: total_jobs,
                pending: jobs_pending,
                training: jobs_training,
                completed: jobs_completed,
                failed: jobs_failed,
            },
            models: ModelStatusCounts {
                total: total_models,
                undeployed: models_undeployed,
                active: models_active,
            },
            evaluations: EvaluationStatusCounts {
                total: total_evals,
                running: evals_running,
                completed: evals_completed,
                failed: evals_failed,
            },
        })
    }
}
