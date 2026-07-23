use chrono::Utc;
use platform_db::models::DataGuide;
use platform_shared::enums::{DataGuideStatus, DocumentStatus, SampleRating, TaskType};
use serde_json::json;
use uuid::Uuid;

use crate::dto::datagen::{
    CreateDataGuideRequest, DataGuideResponse, Facet, GenerateFacetsRequest,
    GeneratePreviewRequest, PreviewSample, RateSamplesRequest, UpdateFacetsRequest,
    UpdateGuidanceRequest,
};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{
    DataGuideRepository, DatasetRepository, DocumentRepository, TenantRepository,
};
use crate::services::plan_service::PlanService;
use crate::temporal::{TraceContext, WorkflowOrchestrator};

/// Default number of candidate facets generated when the caller doesn't specify one.
const DEFAULT_NUM_FACETS: u32 = 8;
/// Default number of preview samples generated when the caller doesn't specify one.
const DEFAULT_NUM_SAMPLES: u32 = 5;
/// Bounds on caller-supplied counts. These fan out into LLM calls (and cost),
/// so an unbounded request must not be passed through to the workers.
const MIN_NUM_FACETS: u32 = 1;
const MAX_NUM_FACETS: u32 = 50;
const MIN_NUM_SAMPLES: u32 = 1;
const MAX_NUM_SAMPLES: u32 = 50;

/// Resolve the requested facet count to a sane, bounded value.
fn resolve_num_facets(requested: Option<u32>) -> u32 {
    requested
        .unwrap_or(DEFAULT_NUM_FACETS)
        .clamp(MIN_NUM_FACETS, MAX_NUM_FACETS)
}

/// Resolve the requested preview-sample count to a sane, bounded value.
fn resolve_num_samples(requested: Option<u32>) -> u32 {
    requested
        .unwrap_or(DEFAULT_NUM_SAMPLES)
        .clamp(MIN_NUM_SAMPLES, MAX_NUM_SAMPLES)
}

/// Business logic for the guided synthetic-data session (data guide).
///
/// Orchestrates the Draft → GeneratingFacets → FacetsReady → GeneratingPreview →
/// Ready → Generating → Completed pipeline, delegating async work to Temporal
/// workflows via `WorkflowOrchestrator` and validating every status transition.
pub struct DataGuideService;

impl DataGuideService {
    /// Fetch the current data guide session for a project (404 if none exists yet).
    pub async fn get_for_project(
        repo: &dyn DataGuideRepository,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> AppResult<DataGuideResponse> {
        let guide =
            repo.get_for_project(tenant_id, project_id)
                .await?
                .ok_or(AppError::NotFound {
                    message: "Data guide not found".to_string(),
                })?;

        Ok(guide.into())
    }

    /// Return the existing data guide for a project, or create a fresh one in `Draft`.
    pub async fn create_or_get(
        repo: &dyn DataGuideRepository,
        tenant_id: Uuid,
        project_id: Uuid,
        req: CreateDataGuideRequest,
    ) -> AppResult<DataGuideResponse> {
        if let Some(existing) = repo.get_for_project(tenant_id, project_id).await? {
            return Ok(existing.into());
        }

        let task_type = req.task_type.unwrap_or(TaskType::QuestionAnswering);
        let guide = repo
            .create(tenant_id, project_id, &task_type.to_string())
            .await?;

        tracing::info!(
            data_guide_id = %guide.id,
            project_id = %project_id,
            tenant_id = %tenant_id,
            "Data guide created"
        );

        Ok(guide.into())
    }

    /// Start facet generation from parsed source documents.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_facets(
        repo: &dyn DataGuideRepository,
        doc_repo: &dyn DocumentRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        project_id: Uuid,
        guide_id: Uuid,
        req: GenerateFacetsRequest,
        trace_ctx: TraceContext,
    ) -> AppResult<DataGuideResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Data guide workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        let mut guide = Self::require_guide(repo, tenant_id, guide_id).await?;
        let current: DataGuideStatus = guide.status.parse().unwrap_or(DataGuideStatus::Draft);

        if !is_valid_transition(current, DataGuideStatus::GeneratingFacets) {
            return Err(AppError::BadRequest {
                message: format!("Cannot generate facets from {current} state"),
            });
        }

        let parsed_docs = doc_repo
            .list_by_status(tenant_id, project_id, DocumentStatus::Parsed)
            .await?;
        if parsed_docs.is_empty() {
            return Err(AppError::BadRequest {
                message: "No parsed documents available to generate facets from".to_string(),
            });
        }

        let num_facets = resolve_num_facets(req.num_facets);

        let result = orchestrator
            .start_generate_facets(
                tenant_id,
                project_id,
                guide_id,
                &guide.task_type,
                &guide.guidance,
                num_facets,
                trace_ctx,
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start facet generation: {e}"))
            })?;

        repo.update_status(
            tenant_id,
            guide_id,
            &DataGuideStatus::GeneratingFacets.to_string(),
        )
        .await?;

        tracing::info!(
            data_guide_id = %guide_id,
            workflow_id = %result.workflow_id,
            "Facet generation started"
        );

        guide.status = DataGuideStatus::GeneratingFacets.to_string();
        Ok(guide.into())
    }

    /// Persist the user's kept/discarded facet selections (data guide stays `FacetsReady`).
    pub async fn update_facets(
        repo: &dyn DataGuideRepository,
        tenant_id: Uuid,
        guide_id: Uuid,
        req: UpdateFacetsRequest,
    ) -> AppResult<DataGuideResponse> {
        let mut guide = Self::require_guide(repo, tenant_id, guide_id).await?;
        let current: DataGuideStatus = guide.status.parse().unwrap_or(DataGuideStatus::Draft);

        if current != DataGuideStatus::FacetsReady {
            return Err(AppError::BadRequest {
                message: format!("Cannot update facets while data guide is in {current} state"),
            });
        }

        let facets_json = serde_json::to_value(&req.facets)
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
        repo.update_facets(tenant_id, guide_id, facets_json.clone())
            .await?;

        guide.facets = facets_json;
        Ok(guide.into())
    }

    /// Start preview-sample generation from the kept facets.
    pub async fn start_preview(
        repo: &dyn DataGuideRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        project_id: Uuid,
        guide_id: Uuid,
        req: GeneratePreviewRequest,
        trace_ctx: TraceContext,
    ) -> AppResult<DataGuideResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Data guide workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        let mut guide = Self::require_guide(repo, tenant_id, guide_id).await?;
        let current: DataGuideStatus = guide.status.parse().unwrap_or(DataGuideStatus::Draft);

        if !is_valid_transition(current, DataGuideStatus::GeneratingPreview) {
            return Err(AppError::BadRequest {
                message: format!("Cannot generate preview from {current} state"),
            });
        }

        let facets: Vec<Facet> = serde_json::from_value(guide.facets.clone()).unwrap_or_default();
        if !facets.iter().any(|f| f.keep) {
            return Err(AppError::BadRequest {
                message: "No kept facets available; generate and review facets first".to_string(),
            });
        }

        let num_samples = resolve_num_samples(req.num_samples);

        let result = orchestrator
            .start_generate_preview(
                tenant_id,
                project_id,
                guide_id,
                &guide.task_type,
                &guide.guidance,
                guide.facets.clone(),
                num_samples,
                trace_ctx,
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start preview generation: {e}"))
            })?;

        repo.update_status(
            tenant_id,
            guide_id,
            &DataGuideStatus::GeneratingPreview.to_string(),
        )
        .await?;

        tracing::info!(
            data_guide_id = %guide_id,
            workflow_id = %result.workflow_id,
            "Preview generation started"
        );

        guide.status = DataGuideStatus::GeneratingPreview.to_string();
        Ok(guide.into())
    }

    /// Merge user ratings into the stored preview samples (data guide stays `Ready`).
    pub async fn rate_samples(
        repo: &dyn DataGuideRepository,
        tenant_id: Uuid,
        guide_id: Uuid,
        req: RateSamplesRequest,
    ) -> AppResult<DataGuideResponse> {
        let mut guide = Self::require_guide(repo, tenant_id, guide_id).await?;
        let current: DataGuideStatus = guide.status.parse().unwrap_or(DataGuideStatus::Draft);

        if current != DataGuideStatus::Ready {
            return Err(AppError::BadRequest {
                message: format!("Cannot rate samples while data guide is in {current} state"),
            });
        }

        let mut samples: Vec<PreviewSample> =
            serde_json::from_value(guide.preview_samples.clone()).unwrap_or_default();
        for item in &req.ratings {
            if let Some(sample) = samples.iter_mut().find(|s| s.id == item.sample_id) {
                sample.rating = Some(item.rating);
            }
        }

        let samples_json =
            serde_json::to_value(&samples).map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;
        repo.apply_ratings(tenant_id, guide_id, samples_json.clone())
            .await?;

        guide.preview_samples = samples_json;
        Ok(guide.into())
    }

    /// Regenerate guidance text from user-rated preview samples.
    pub async fn refine_guidance(
        repo: &dyn DataGuideRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        project_id: Uuid,
        guide_id: Uuid,
        trace_ctx: TraceContext,
    ) -> AppResult<DataGuideResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Data guide workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        let mut guide = Self::require_guide(repo, tenant_id, guide_id).await?;
        let current: DataGuideStatus = guide.status.parse().unwrap_or(DataGuideStatus::Draft);

        if !is_valid_transition(current, DataGuideStatus::GeneratingPreview) {
            return Err(AppError::BadRequest {
                message: format!("Cannot refine guidance from {current} state"),
            });
        }

        let samples: Vec<PreviewSample> =
            serde_json::from_value(guide.preview_samples.clone()).unwrap_or_default();
        if samples.iter().all(|s| s.rating.is_none()) {
            return Err(AppError::BadRequest {
                message: "Rate at least one sample before refining guidance".to_string(),
            });
        }

        // The Python `RefineGuidanceActivity` deserializes `rated` into `RatedSample`
        // (`{prompt, response, looks_good: bool}`) — it has no `rating` field and no
        // notion of an unrated sample, so only rated samples are included and the
        // three-state `rating` is collapsed to the boolean the refiner expects.
        let rated: Vec<serde_json::Value> = samples
            .iter()
            .filter_map(|s| {
                s.rating.map(|rating| {
                    json!({
                        "prompt": s.prompt,
                        "response": s.response,
                        "looks_good": rating == SampleRating::Realistic,
                    })
                })
            })
            .collect();

        let result = orchestrator
            .start_refine_guidance(
                tenant_id,
                project_id,
                guide_id,
                &guide.task_type,
                &guide.guidance,
                serde_json::to_value(&rated).map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?,
                trace_ctx,
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to start guidance refinement: {e}"))
            })?;

        repo.update_status(
            tenant_id,
            guide_id,
            &DataGuideStatus::GeneratingPreview.to_string(),
        )
        .await?;

        tracing::info!(
            data_guide_id = %guide_id,
            workflow_id = %result.workflow_id,
            "Guidance refinement started"
        );

        guide.status = DataGuideStatus::GeneratingPreview.to_string();
        Ok(guide.into())
    }

    /// Overwrite the free-form guidance text (allowed while editable, i.e. not mid-generation).
    pub async fn update_guidance(
        repo: &dyn DataGuideRepository,
        tenant_id: Uuid,
        guide_id: Uuid,
        req: UpdateGuidanceRequest,
    ) -> AppResult<DataGuideResponse> {
        let mut guide = Self::require_guide(repo, tenant_id, guide_id).await?;
        let current: DataGuideStatus = guide.status.parse().unwrap_or(DataGuideStatus::Draft);

        if matches!(
            current,
            DataGuideStatus::GeneratingFacets
                | DataGuideStatus::GeneratingPreview
                | DataGuideStatus::Generating
                | DataGuideStatus::Completed
                | DataGuideStatus::Failed
        ) {
            return Err(AppError::BadRequest {
                message: format!("Cannot edit guidance while data guide is in {current} state"),
            });
        }

        let mut history: Vec<serde_json::Value> =
            serde_json::from_value(guide.refinement_history.clone()).unwrap_or_default();
        history.push(json!({ "guidance": guide.guidance, "replaced_at": Utc::now() }));
        let history_json =
            serde_json::to_value(&history).map_err(|e| AppError::Internal(anyhow::anyhow!(e)))?;

        repo.update_guidance(tenant_id, guide_id, &req.guidance, history_json)
            .await?;

        guide.guidance = req.guidance;
        Ok(guide.into())
    }

    /// Start full dataset generation from the finalized guidance and facets.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub async fn generate_dataset(
        repo: &dyn DataGuideRepository,
        doc_repo: &dyn DocumentRepository,
        dataset_repo: &dyn DatasetRepository,
        tenant_repo: &dyn TenantRepository,
        orchestrator: Option<&dyn WorkflowOrchestrator>,
        tenant_id: Uuid,
        project_id: Uuid,
        guide_id: Uuid,
        trace_ctx: TraceContext,
    ) -> AppResult<DataGuideResponse> {
        let orchestrator = orchestrator.ok_or(AppError::BadRequest {
            message: "Data guide workflows are not available (orchestrator not configured)"
                .to_string(),
        })?;

        let mut guide = Self::require_guide(repo, tenant_id, guide_id).await?;
        let current: DataGuideStatus = guide.status.parse().unwrap_or(DataGuideStatus::Draft);

        if !is_valid_transition(current, DataGuideStatus::Generating) {
            return Err(AppError::BadRequest {
                message: format!("Cannot generate dataset from {current} state"),
            });
        }

        let current_pairs = dataset_repo.sum_pair_count(tenant_id).await?;
        PlanService::check_training_pairs_limit(tenant_repo, tenant_id, current_pairs).await?;

        let parsed_docs = doc_repo
            .list_by_status(tenant_id, project_id, DocumentStatus::Parsed)
            .await?;
        if parsed_docs.is_empty() {
            return Err(AppError::BadRequest {
                message: "No parsed documents available for dataset generation".to_string(),
            });
        }
        let doc_ids: Vec<Uuid> = parsed_docs.iter().map(|d| d.id).collect();

        let result = orchestrator
            .start_generate_dataset(
                tenant_id,
                project_id,
                guide_id,
                &guide.task_type,
                &guide.guidance,
                guide.facets.clone(),
                doc_ids,
                trace_ctx,
            )
            .await
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!(
                    "Failed to start GenerateDatasetWorkflow: {e}"
                ))
            })?;

        repo.update_status(
            tenant_id,
            guide_id,
            &DataGuideStatus::Generating.to_string(),
        )
        .await?;

        tracing::info!(
            data_guide_id = %guide_id,
            workflow_id = %result.workflow_id,
            "Dataset generation started"
        );

        guide.status = DataGuideStatus::Generating.to_string();
        Ok(guide.into())
    }

    async fn require_guide(
        repo: &dyn DataGuideRepository,
        tenant_id: Uuid,
        guide_id: Uuid,
    ) -> AppResult<DataGuide> {
        repo.get(tenant_id, guide_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Data guide not found".to_string(),
            })
    }
}

/// Validate data guide status transitions.
///
/// Forward flow: Draft → GeneratingFacets → FacetsReady → GeneratingPreview → Ready
/// → Generating → Completed. `Ready` can loop back into GeneratingPreview (re-preview)
/// or GeneratingFacets (regenerate facets); FacetsReady can also regenerate facets.
/// Any in-flight Generating* state can end in `Failed`.
fn is_valid_transition(from: DataGuideStatus, to: DataGuideStatus) -> bool {
    use DataGuideStatus::*;
    matches!(
        (from, to),
        (Draft, GeneratingFacets)
            | (GeneratingFacets, FacetsReady)
            | (FacetsReady, GeneratingPreview)
            | (GeneratingPreview, Ready)
            | (Ready, GeneratingPreview)
            | (Ready, Generating)
            | (Generating, Completed)
            | (GeneratingFacets, Failed)
            | (GeneratingPreview, Failed)
            | (Generating, Failed)
            | (FacetsReady, GeneratingFacets)
            | (Ready, GeneratingFacets)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_resolution_applies_defaults_and_bounds() {
        // Unspecified → default.
        assert_eq!(resolve_num_facets(None), DEFAULT_NUM_FACETS);
        assert_eq!(resolve_num_samples(None), DEFAULT_NUM_SAMPLES);
        // In-range values pass through.
        assert_eq!(resolve_num_facets(Some(12)), 12);
        assert_eq!(resolve_num_samples(Some(3)), 3);
        // Over/under the bounds are clamped, not passed through.
        assert_eq!(resolve_num_facets(Some(1_000_000)), MAX_NUM_FACETS);
        assert_eq!(resolve_num_facets(Some(0)), MIN_NUM_FACETS);
        assert_eq!(resolve_num_samples(Some(1_000_000)), MAX_NUM_SAMPLES);
        assert_eq!(resolve_num_samples(Some(0)), MIN_NUM_SAMPLES);
    }

    #[test]
    fn transition_rules() {
        use platform_shared::enums::DataGuideStatus::*;
        assert!(is_valid_transition(Draft, GeneratingFacets));
        assert!(is_valid_transition(FacetsReady, GeneratingPreview));
        assert!(is_valid_transition(Ready, Generating));
        assert!(!is_valid_transition(Draft, Completed));
        assert!(!is_valid_transition(Generating, GeneratingFacets));
    }

    #[test]
    fn forward_pipeline_transitions_are_valid() {
        use DataGuideStatus::*;
        let forward = [
            (Draft, GeneratingFacets),
            (GeneratingFacets, FacetsReady),
            (FacetsReady, GeneratingPreview),
            (GeneratingPreview, Ready),
            (Ready, Generating),
            (Generating, Completed),
        ];
        for (from, to) in forward {
            assert!(
                is_valid_transition(from, to),
                "Expected {from} → {to} to be valid"
            );
        }
    }

    #[test]
    fn regenerate_and_re_preview_transitions_are_valid() {
        use DataGuideStatus::*;
        let allowed = [
            (Ready, GeneratingPreview),
            (FacetsReady, GeneratingFacets),
            (Ready, GeneratingFacets),
        ];
        for (from, to) in allowed {
            assert!(
                is_valid_transition(from, to),
                "Expected {from} → {to} to be valid"
            );
        }
    }

    #[test]
    fn any_generating_state_can_fail() {
        use DataGuideStatus::*;
        for from in [GeneratingFacets, GeneratingPreview, Generating] {
            assert!(
                is_valid_transition(from, Failed),
                "Expected {from} → Failed to be valid"
            );
        }
    }

    #[test]
    fn terminal_states_have_no_outgoing_transitions() {
        use DataGuideStatus::*;
        let all_targets = [
            Draft,
            GeneratingFacets,
            FacetsReady,
            GeneratingPreview,
            Ready,
            Generating,
            Completed,
            Failed,
        ];
        for to in all_targets {
            assert!(
                !is_valid_transition(Completed, to),
                "Completed → {to} should be invalid"
            );
            assert!(
                !is_valid_transition(Failed, to),
                "Failed → {to} should be invalid"
            );
        }
    }

    #[test]
    fn same_status_transition_is_invalid() {
        use DataGuideStatus::*;
        for status in [Draft, FacetsReady, Ready, Generating, Completed, Failed] {
            assert!(
                !is_valid_transition(status, status),
                "Expected {status} → {status} to be invalid"
            );
        }
    }
}
