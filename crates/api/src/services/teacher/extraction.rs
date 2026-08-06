//! Admitting a high-fidelity distill run.
//!
//! Joins the three things that must all hold before a hosted teacher is allowed
//! to occupy a GPU: the dataset's own teacher must be one we can run, the
//! student must plausibly share its tokenizer, and the estimated cost must fit
//! inside the tenant's teacher-GPU budget. Nothing here starts work — it decides
//! whether starting work is permitted, and returns the config the extraction job
//! will run under.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;
use uuid::Uuid;

use platform_db::models::Dataset;
use platform_shared::enums::{DistillMethod, TeacherPrecision};

use crate::error::{AppError, AppResult};
use crate::repositories::traits::BillingEventRepository;
use crate::services::teacher::billing::check_teacher_gpu_spend_cap;
use crate::services::teacher::cost::{ExtractionEstimate, estimate_extraction, scored_tokens_for};
use crate::services::teacher::fidelity::{clamp_top_k, hosted_scorer_for};
use crate::services::teacher::hosted::HostedTeacherEntry;

/// Fidelity options a caller may set on a distill run. Absent means the Stage 1
/// text path, which is the default everywhere.
#[derive(Debug, Clone, Default, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DistillOptionsDto {
    #[ts(optional)]
    pub method: Option<DistillMethod>,
    #[ts(optional)]
    pub precision: Option<TeacherPrecision>,
    #[ts(optional)]
    pub top_k_logprobs: Option<u32>,
}

impl DistillOptionsDto {
    pub fn wants_logits(&self) -> bool {
        self.method == Some(DistillMethod::Logit)
    }
}

/// Everything the extraction job needs, resolved from the catalog rather than
/// from the request: a tenant cannot name the model we execute, only ask for the
/// one its own data already came from.
#[derive(Debug, Clone, Serialize)]
pub struct ExtractionPlan {
    pub teacher_model: String,
    pub teacher_revision: String,
    pub precision: TeacherPrecision,
    pub top_k_logprobs: u32,
    pub gpu_class: String,
    pub estimate: ExtractionEstimate,
}

impl ExtractionPlan {
    /// Block persisted on the training job and handed to the workflow. Private
    /// so `attach_to_teacher_config` stays the only way this reaches a column.
    fn workflow_value(&self) -> serde_json::Value {
        serde_json::json!({
            "distill_method": DistillMethod::Logit.to_string(),
            "teacher_model": self.teacher_model,
            "teacher_revision": self.teacher_revision,
            "precision": self.precision.to_string(),
            "top_k_logprobs": self.top_k_logprobs,
            "gpu_class": self.gpu_class,
            "est_cost_usd": self.estimate.est_cost_usd,
            "est_gpu_hours": self.estimate.est_gpu_hours,
        })
    }
}

/// Fold an admitted extraction plan into a job's `teacher` block.
///
/// The plan is recorded alongside the teacher rather than in a column of its
/// own, because it is entirely a statement about this job's teacher — and this
/// module owns that column, so the merge happens here rather than at the call
/// site. A job with no fidelity upgrade is returned untouched.
///
/// An admitted plan with no block to live on is an error rather than a plan
/// dropped: the tenant has already been quoted for the teacher's GPU pass and
/// admitted against their teacher budget, so a run that proceeded without the plan
/// would be charged for scoring that never happened.
pub fn attach_to_teacher_config(
    teacher_config: Option<serde_json::Value>,
    plan: Option<&ExtractionPlan>,
) -> AppResult<Option<serde_json::Value>> {
    match (teacher_config, plan) {
        (Some(mut block), Some(plan)) => {
            block["extraction"] = plan.workflow_value();
            Ok(Some(block))
        }
        (None, Some(_)) => Err(AppError::Internal(anyhow::anyhow!(
            "an extraction plan was admitted for a job with no teacher provenance"
        ))),
        (block, None) => Ok(block),
    }
}

/// Price a fidelity upgrade for `dataset` and `student_model`.
///
/// Separate from admission so the same arithmetic backs both the quoted estimate
/// the user sees and the cap check that follows — a quote produced by different
/// code from the charge is a quote that eventually disagrees with the bill.
pub fn plan_extraction(
    dataset: &Dataset,
    student_model: &str,
    options: &DistillOptionsDto,
    gpu_hourly_rate_for: impl Fn(&str) -> f64,
) -> Result<ExtractionPlan, &'static str> {
    let entry: &HostedTeacherEntry =
        hosted_scorer_for(&dataset.config, student_model).map_err(|blocker| blocker.message())?;
    let gpu_class = entry.gpu_class.to_string();
    let (scored_tokens, basis) =
        scored_tokens_for(dataset.scored_completion_tokens, dataset.pair_count);

    Ok(ExtractionPlan {
        teacher_model: entry.model_id.to_string(),
        teacher_revision: entry.revision.to_string(),
        precision: options.precision.unwrap_or_default(),
        top_k_logprobs: clamp_top_k(options.top_k_logprobs),
        estimate: estimate_extraction(
            scored_tokens,
            basis,
            entry.est_scored_tokens_per_sec,
            gpu_hourly_rate_for(&gpu_class),
            &gpu_class,
        ),
        gpu_class,
    })
}

/// Refuse the run when its estimate would exceed the tenant's teacher-GPU cap.
///
/// Checked before the job row is created, so a refused run leaves nothing
/// half-started for an operator to clean up.
pub async fn admit_extraction(
    billing_repo: &dyn BillingEventRepository,
    tenant_id: Uuid,
    cap: Option<f64>,
    plan: &ExtractionPlan,
) -> AppResult<()> {
    check_teacher_gpu_spend_cap(billing_repo, tenant_id, cap, plan.estimate.est_cost_usd).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn dataset(teacher_model: &str, scored: Option<i64>) -> Dataset {
        Dataset {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            name: "d".to_string(),
            storage_path: None,
            format: "chatml".to_string(),
            status: "approved".to_string(),
            pair_count: Some(1000),
            stats: json!({}),
            config: json!({"teacher": {
                "host": "inference.example.com",
                "model": teacher_model,
                "policy": "allowed",
                "cot": false,
            }}),
            error: None,
            prompt_tokens: None,
            completion_tokens: None,
            scored_completion_tokens: scored,
            token_count_tokenizer_hash: None,
            size_bytes: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn rate(_class: &str) -> f64 {
        3.00
    }

    #[test]
    fn eligible_pair_is_planned_from_the_catalog() {
        let plan = plan_extraction(
            &dataset("Qwen/Qwen3-32B", Some(1_200_000)),
            "Qwen/Qwen3-8B",
            &DistillOptionsDto::default(),
            rate,
        )
        .expect("should plan");

        assert_eq!(plan.teacher_model, "Qwen/Qwen3-32B");
        assert_eq!(plan.teacher_revision.len(), 40);
        assert_eq!(plan.precision, TeacherPrecision::Bf16);
        assert_eq!(plan.gpu_class, "a10080gb");
        assert!(plan.estimate.est_cost_usd > 0.0);
    }

    #[test]
    fn ineligible_pair_returns_the_user_facing_reason() {
        let reason = plan_extraction(
            &dataset("Qwen/Qwen3-32B", None),
            "unsloth/Llama-3.2-1B-Instruct",
            &DistillOptionsDto::default(),
            rate,
        )
        .expect_err("family mismatch must not plan");

        assert!(reason.contains("read text differently"));
    }

    #[test]
    fn api_only_teacher_cannot_be_run_by_us() {
        assert!(
            plan_extraction(
                &dataset("some-proprietary-model", None),
                "Qwen/Qwen3-8B",
                &DistillOptionsDto::default(),
                rate,
            )
            .is_err()
        );
    }

    #[test]
    fn caller_top_k_is_clamped_into_range() {
        let plan = plan_extraction(
            &dataset("Qwen/Qwen3-32B", Some(1000)),
            "Qwen/Qwen3-8B",
            &DistillOptionsDto {
                top_k_logprobs: Some(100_000),
                ..Default::default()
            },
            rate,
        )
        .unwrap();

        assert_eq!(
            plan.top_k_logprobs,
            crate::services::teacher::fidelity::MAX_TOP_K_LOGPROBS
        );
    }

    #[test]
    fn workflow_value_names_the_catalog_model_not_the_request() {
        let plan = plan_extraction(
            &dataset("Qwen/Qwen3-32B", Some(1000)),
            "Qwen/Qwen3-8B",
            &DistillOptionsDto::default(),
            rate,
        )
        .unwrap();
        let block = plan.workflow_value();

        assert_eq!(block["teacher_model"], "Qwen/Qwen3-32B");
        assert_eq!(block["distill_method"], "logit");
        assert_eq!(block["precision"], "bf16");
    }

    fn logit_plan() -> ExtractionPlan {
        plan_extraction(
            &dataset("Qwen/Qwen3-32B", Some(1000)),
            "Qwen/Qwen3-8B",
            &DistillOptionsDto {
                method: Some(DistillMethod::Logit),
                ..Default::default()
            },
            rate,
        )
        .unwrap()
    }

    #[test]
    fn attaching_a_plan_preserves_the_teacher_it_is_merged_into() {
        let teacher = json!({"host": "inference.example.com", "model": "Qwen/Qwen3-32B"});

        let merged = attach_to_teacher_config(Some(teacher), Some(&logit_plan()))
            .unwrap()
            .unwrap();

        assert_eq!(merged["host"], "inference.example.com");
        assert_eq!(merged["model"], "Qwen/Qwen3-32B");
        assert_eq!(merged["extraction"]["distill_method"], "logit");
    }

    /// A text-path distill run must not gain an extraction block, or the worker
    /// would look for teacher artifacts that were never produced.
    #[test]
    fn no_plan_leaves_the_teacher_block_untouched() {
        let teacher = json!({"model": "Qwen/Qwen3-32B"});

        let merged = attach_to_teacher_config(Some(teacher.clone()), None)
            .unwrap()
            .unwrap();

        assert_eq!(merged, teacher);
    }

    #[test]
    fn a_job_without_a_teacher_stays_without_one() {
        assert!(attach_to_teacher_config(None, None).unwrap().is_none());
    }

    /// The plan has been priced and admitted against the teacher-GPU budget by the
    /// time this runs, so there is no correct way to continue without it: the run
    /// would train as plain SFT on hardware the tenant is paying a teacher rate for.
    /// Returning the job unchanged is what made that silent.
    #[test]
    fn an_admitted_plan_with_nowhere_to_live_is_an_error_not_a_dropped_plan() {
        assert!(attach_to_teacher_config(None, Some(&logit_plan())).is_err());
    }

    #[test]
    fn options_default_to_the_text_path() {
        assert!(!DistillOptionsDto::default().wants_logits());
        assert!(
            DistillOptionsDto {
                method: Some(DistillMethod::Logit),
                ..Default::default()
            }
            .wants_logits()
        );
        assert!(
            !DistillOptionsDto {
                method: Some(DistillMethod::Text),
                ..Default::default()
            }
            .wants_logits()
        );
    }

    #[test]
    fn measured_token_counts_change_the_estimate() {
        let cheap = plan_extraction(
            &dataset("Qwen/Qwen3-32B", Some(1000)),
            "Qwen/Qwen3-8B",
            &DistillOptionsDto::default(),
            rate,
        )
        .unwrap();
        let dear = plan_extraction(
            &dataset("Qwen/Qwen3-32B", Some(50_000_000)),
            "Qwen/Qwen3-8B",
            &DistillOptionsDto::default(),
            rate,
        )
        .unwrap();

        assert!(dear.estimate.est_cost_usd > cheap.estimate.est_cost_usd);
    }
}
