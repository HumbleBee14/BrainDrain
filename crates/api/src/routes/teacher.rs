//! Teacher picker endpoints for distillation setup.
//!
//! `catalog` lists the curated open teacher models; `classify` returns the
//! provider policy for a chosen endpoint + model so the UI can render the
//! policy badge before anything is launched.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::error::{AppError, AppResult};
use crate::services::teacher::cost::{ExtractionEstimate, estimate_extraction, scored_tokens_for};
use crate::services::teacher::fidelity::{clamp_top_k, hosted_scorer_for};
use crate::services::teacher::on_policy::plan_on_policy;
use crate::services::teacher::policy::{
    ProviderPolicy, TeacherCatalogEntry, classify_provider, teacher_catalog,
};
use crate::services::tenant_settings_service::TenantSettingsService;
use crate::services::training_job_service::resolve_gpu_rate;
use platform_shared::enums::TrainingMode;

/// Shown where a model simply has no teacher behind it — the common case, and not
/// a problem the user needs to solve.
const NOT_DISTILLED_MESSAGE: &str =
    "Only models distilled from a teacher can be sharpened against one.";

/// The dataset carried the teacher's identity, so without it there is nothing to
/// re-derive which teacher to boot.
const DATASET_GONE_MESSAGE: &str = "The dataset this model was trained on is no longer available, so its teacher cannot be identified.";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/teachers/catalog", get(get_teacher_catalog))
        .route("/teachers/classify", post(classify_teacher))
        .route("/teachers/cost-estimate", post(estimate_teacher_cost))
        .route("/models/{id}/improve-offer", get(get_improve_offer))
}

/// Whether a model can be sharpened against its own teacher, and what that costs.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ImproveOfferResponse {
    pub eligible: bool,
    /// Why not, in the user's terms. Absent when eligible.
    #[ts(optional)]
    pub reason: Option<String>,
    #[ts(optional)]
    pub teacher_model: Option<String>,
    #[ts(optional)]
    pub dataset_id: Option<String>,
    #[ts(optional)]
    pub estimate: Option<ExtractionEstimate>,
}

impl ImproveOfferResponse {
    fn ineligible(reason: impl Into<String>) -> Self {
        Self {
            eligible: false,
            reason: Some(reason.into()),
            teacher_model: None,
            dataset_id: None,
            estimate: None,
        }
    }
}

/// Offer an on-policy improve pass for a trained model, or explain why not.
///
/// Ineligibility is a 200 with a reason, not an error: "this model cannot be
/// sharpened" is a normal answer for most models, and the card that asks simply
/// does not render. Only a missing model is a 404.
#[utoipa::path(
    get,
    path = "/api/v1/models/{id}/improve-offer",
    tag = "Training",
    params(("id" = String, Path, description = "Model id")),
    responses((status = 200, description = "Improve-pass offer", body = ImproveOfferResponse)),
    security(("jwt" = []))
)]
pub async fn get_improve_offer(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> AppResult<Json<ImproveOfferResponse>> {
    let model_id = id.parse::<Uuid>().map_err(|_| AppError::BadRequest {
        message: "Invalid model id format".to_string(),
    })?;
    let model = state
        .model_repo()
        .get_by_id(user.tenant_id, model_id)
        .await?
        .ok_or(AppError::NotFound {
            message: "Model not found".to_string(),
        })?;

    let Some(job) = state
        .training_job_repo()
        .get_by_id(user.tenant_id, model.training_job_id)
        .await?
    else {
        return Ok(Json(ImproveOfferResponse::ineligible(
            NOT_DISTILLED_MESSAGE,
        )));
    };

    // Only a distilled model has a teacher to be sharpened against. Everything
    // else — including a model whose training job predates distillation — is a
    // plain "not applicable" rather than a failure.
    if job.mode.parse::<TrainingMode>().ok() != Some(TrainingMode::Distill) {
        return Ok(Json(ImproveOfferResponse::ineligible(
            NOT_DISTILLED_MESSAGE,
        )));
    }

    let Some(dataset) = state
        .dataset_repo()
        .get_by_id(user.tenant_id, job.dataset_id)
        .await?
    else {
        return Ok(Json(ImproveOfferResponse::ineligible(DATASET_GONE_MESSAGE)));
    };

    let admin_config =
        TenantSettingsService::get_admin_config(state.tenant_repo(), user.tenant_id).await?;
    let epochs = job
        .hyperparams
        .get("num_train_epochs")
        .and_then(|v| v.as_i64());

    match plan_on_policy(
        &dataset,
        &model.base_model,
        epochs,
        state.config().on_policy_tokens_per_sec,
        |gpu_class| resolve_gpu_rate(&admin_config.gpu_rates, gpu_class),
    ) {
        Ok(plan) => Ok(Json(ImproveOfferResponse {
            eligible: true,
            reason: None,
            teacher_model: Some(plan.teacher_model),
            dataset_id: Some(job.dataset_id.to_string()),
            estimate: Some(plan.estimate),
        })),
        Err(reason) => Ok(Json(ImproveOfferResponse::ineligible(reason))),
    }
}

/// Request to classify a teacher choice. Deliberately excludes the API key —
/// classification needs only the endpoint and model.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ClassifyTeacherRequest {
    pub api_base_url: String,
    pub model: String,
}

#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ClassifyTeacherResponse {
    pub policy: ProviderPolicy,
}

/// List the curated open teacher models (permissive licenses).
#[utoipa::path(
    get,
    path = "/api/v1/teachers/catalog",
    tag = "Training",
    responses((status = 200, description = "Curated teacher models", body = [TeacherCatalogEntry])),
    security(("jwt" = []))
)]
pub async fn get_teacher_catalog(
    _user: AuthenticatedUser,
) -> AppResult<Json<Vec<TeacherCatalogEntry>>> {
    Ok(Json(teacher_catalog().to_vec()))
}

/// Classify a teacher endpoint + model into a provider policy.
#[utoipa::path(
    post,
    path = "/api/v1/teachers/classify",
    tag = "Training",
    request_body = ClassifyTeacherRequest,
    responses((status = 200, description = "Provider policy", body = ClassifyTeacherResponse)),
    security(("jwt" = []))
)]
pub async fn classify_teacher(
    _user: AuthenticatedUser,
    Json(body): Json<ClassifyTeacherRequest>,
) -> AppResult<Json<ClassifyTeacherResponse>> {
    Ok(Json(ClassifyTeacherResponse {
        policy: classify_provider(&body.api_base_url, &body.model),
    }))
}

/// Ask whether a dataset can be trained at higher fidelity, and what it costs.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct TeacherCostEstimateRequest {
    pub dataset_id: String,
    /// The student that would train on the artifacts. Tokenizer compatibility is
    /// judged against this model.
    pub student_model: String,
    #[ts(optional)]
    pub top_k_logprobs: Option<u32>,
}

/// Whether the fidelity upgrade is on offer for a dataset, and its price.
///
/// `eligible: false` always carries a `reason` written for the user; the UI
/// shows nothing rather than a disabled control it cannot explain.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct TeacherCostEstimateResponse {
    pub eligible: bool,
    #[ts(optional)]
    pub reason: Option<String>,
    #[ts(optional)]
    pub teacher_model: Option<String>,
    #[ts(optional)]
    pub top_k_logprobs: Option<u32>,
    #[ts(optional)]
    pub estimate: Option<ExtractionEstimate>,
}

/// Estimate the GPU cost of scoring a dataset with a hosted teacher.
#[utoipa::path(
    post,
    path = "/api/v1/teachers/cost-estimate",
    tag = "Training",
    request_body = TeacherCostEstimateRequest,
    responses((status = 200, description = "Fidelity eligibility and cost", body = TeacherCostEstimateResponse)),
    security(("jwt" = []))
)]
pub async fn estimate_teacher_cost(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(body): Json<TeacherCostEstimateRequest>,
) -> AppResult<Json<TeacherCostEstimateResponse>> {
    let dataset_id = body
        .dataset_id
        .parse::<Uuid>()
        .map_err(|_| AppError::BadRequest {
            message: "Invalid dataset_id format".to_string(),
        })?;
    let dataset = state
        .dataset_repo()
        .get_by_id(user.tenant_id, dataset_id)
        .await?
        .ok_or(AppError::NotFound {
            message: "Dataset not found".to_string(),
        })?;

    let entry = match hosted_scorer_for(&dataset.config, &body.student_model) {
        Ok(entry) => entry,
        Err(blocker) => {
            return Ok(Json(TeacherCostEstimateResponse {
                eligible: false,
                reason: Some(blocker.message().to_string()),
                teacher_model: None,
                top_k_logprobs: None,
                estimate: None,
            }));
        }
    };

    let gpu_class = entry.gpu_class.to_string();
    let admin_config =
        TenantSettingsService::get_admin_config(state.tenant_repo(), user.tenant_id).await?;
    let (scored_tokens, basis) =
        scored_tokens_for(dataset.scored_completion_tokens, dataset.pair_count);

    Ok(Json(TeacherCostEstimateResponse {
        eligible: true,
        reason: None,
        teacher_model: Some(entry.model_id.to_string()),
        top_k_logprobs: Some(clamp_top_k(body.top_k_logprobs)),
        estimate: Some(estimate_extraction(
            scored_tokens,
            basis,
            entry.est_scored_tokens_per_sec,
            resolve_gpu_rate(&admin_config.gpu_rates, &gpu_class),
            &gpu_class,
        )),
    }))
}
