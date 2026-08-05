//! Admitting an on-policy improve pass.
//!
//! Shares Stage 2's eligibility rules — the dataset's own teacher must be one we
//! can run, and the student must read text identically — because both stages need
//! the same teacher for the same reason. What differs is what the GPU is for:
//! there is no scoring pass to run ahead of training, since the text being graded
//! is written by the student during the run. The teacher instead occupies a card
//! inside the training container for the run's whole duration, which is why an
//! on-policy job is admitted onto a multi-device GPU class.

use platform_db::models::Dataset;
use platform_shared::enums::{DistillMethod, GpuClass, TeacherPrecision};
use serde::Serialize;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::repositories::traits::BillingEventRepository;
use crate::services::teacher::billing::check_teacher_gpu_spend_cap;
use crate::services::teacher::cost::{EstimateBasis, ExtractionEstimate, estimate_extraction};
use crate::services::teacher::extraction::DistillOptionsDto;
use crate::services::teacher::fidelity::hosted_scorer_for;

/// Completion tokens an improve pass generates per training example, per epoch.
///
/// The student writes a fresh answer for every prompt it trains on, so this is the
/// rollout budget rather than a property of the dataset. Matches the trainer's own
/// `max_completion_length` default; a run that generates shorter answers costs
/// less than quoted, which is the direction an estimate should err in.
const APPROX_ROLLOUT_TOKENS_PER_EXAMPLE: i64 = 512;

/// Epochs assumed when a request does not say, matching `merge_hyperparams` — the
/// number a job actually gets when the caller supplies none. Quoting anything
/// lower here would under-price every default run, which is the one direction an
/// estimate must never err in.
pub const DEFAULT_EPOCHS: i64 = crate::services::training_job_service::DEFAULT_NUM_TRAIN_EPOCHS;

/// Whether these options ask for an improve pass.
pub fn wants_on_policy(options: &DistillOptionsDto) -> bool {
    options.method == Some(DistillMethod::OnPolicy)
}

/// Everything an on-policy run needs, resolved from the catalog and the dataset's
/// provenance rather than from the request.
#[derive(Debug, Clone, Serialize)]
pub struct OnPolicyPlan {
    pub teacher_model: String,
    pub teacher_revision: String,
    pub precision: TeacherPrecision,
    pub gpu_class: String,
    /// The epoch count this quote was computed from. Travels to the worker so the
    /// run trains the number of passes the tenant was charged for — every rollout
    /// is generated token by token, so an extra epoch is an extra teacher-hour.
    pub epochs: i64,
    /// The adapter this run continues training. An improve pass that started from
    /// the bare base model would discard everything the parent learned and grade
    /// rollouts written by an untrained student.
    pub parent_adapter_path: String,
    pub estimate: ExtractionEstimate,
}

impl OnPolicyPlan {
    /// Block persisted on the training job and handed to the workflow. Private so
    /// `attach_to_teacher_config` stays the only way this reaches a column.
    fn workflow_value(&self) -> serde_json::Value {
        serde_json::json!({
            "distill_method": DistillMethod::OnPolicy.to_string(),
            "teacher_model": self.teacher_model,
            "teacher_revision": self.teacher_revision,
            "precision": self.precision.to_string(),
            "gpu_class": self.gpu_class,
            "epochs": self.epochs,
            "parent_adapter_path": self.parent_adapter_path,
            "est_cost_usd": self.estimate.est_cost_usd,
            "est_gpu_hours": self.estimate.est_gpu_hours,
        })
    }
}

/// Fold an admitted improve pass into a job's `teacher` block.
///
/// Shares the `extraction` key with Stage 2 rather than adding a second one: the
/// workflow reads one block and dispatches on the `distill_method` inside it, so a
/// job can never carry two conflicting fidelity plans at once.
///
/// An admitted plan with no block to live on is an error rather than a plan
/// dropped: by this point the tenant has been quoted a two-card price and admitted
/// against their teacher budget, so a run that proceeded without the plan would be
/// charged for a teacher it never started.
pub fn attach_to_teacher_config(
    teacher_config: Option<serde_json::Value>,
    plan: Option<&OnPolicyPlan>,
) -> AppResult<Option<serde_json::Value>> {
    match (teacher_config, plan) {
        (Some(mut block), Some(plan)) => {
            if plan.parent_adapter_path.is_empty() {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "an on-policy plan was admitted with no parent adapter to continue from"
                )));
            }
            block["extraction"] = plan.workflow_value();
            Ok(Some(block))
        }
        (None, Some(_)) => Err(AppError::Internal(anyhow::anyhow!(
            "an on-policy plan was admitted for a job with no teacher provenance"
        ))),
        (block, None) => Ok(block),
    }
}

/// The multi-device class an improve pass runs on.
///
/// Derived from the class the teacher alone needs, because the teacher is the
/// constraint: it wants a whole card of its own, and the student needs another
/// beside it. A teacher small enough for a modest card still gets the paired class
/// for that card, not a shared one.
pub fn paired_gpu_class(teacher_class: GpuClass) -> GpuClass {
    match teacher_class {
        GpuClass::H100 | GpuClass::H100Dual => GpuClass::H100Dual,
        _ => GpuClass::A10080gbDual,
    }
}

/// Tokens an improve pass will generate and score, and how confident that is.
///
/// Always approximate: the count depends on how long the student's own answers
/// turn out to be, which is unknowable before it writes them. Reported as such so
/// the UI can say so rather than implying a measured figure.
pub fn rollout_tokens_for(pair_count: Option<i32>, epochs: i64) -> (i64, EstimateBasis) {
    let pairs = pair_count.unwrap_or(0).max(0) as i64;
    let epochs = epochs.max(1);
    (
        pairs * APPROX_ROLLOUT_TOKENS_PER_EXAMPLE * epochs,
        EstimateBasis::Approximate,
    )
}

/// Price an improve pass for `dataset` and `student_model`.
///
/// Separate from admission so the same arithmetic backs both the quote the user
/// sees and the cap check that follows.
pub fn plan_on_policy(
    dataset: &Dataset,
    student_model: &str,
    epochs: Option<i64>,
    parent_adapter_path: &str,
    tokens_per_sec: f64,
    gpu_hourly_rate_for: impl Fn(&str) -> f64,
) -> Result<OnPolicyPlan, &'static str> {
    let entry = hosted_scorer_for(&dataset.config, student_model).map_err(|blocker| {
        // `&'static str` rather than the blocker itself: callers map this onto a
        // 400 body, and the blocker's own message is already the user-facing copy.
        blocker.message()
    })?;

    let gpu_class = paired_gpu_class(entry.gpu_class).to_string();
    let epochs = epochs.unwrap_or(DEFAULT_EPOCHS).max(1);
    let (tokens, basis) = rollout_tokens_for(dataset.pair_count, epochs);

    Ok(OnPolicyPlan {
        teacher_model: entry.model_id.to_string(),
        teacher_revision: entry.revision.to_string(),
        precision: TeacherPrecision::default(),
        epochs,
        parent_adapter_path: parent_adapter_path.to_string(),
        estimate: estimate_extraction(
            tokens,
            basis,
            tokens_per_sec,
            gpu_hourly_rate_for(&gpu_class),
            &gpu_class,
        ),
        gpu_class,
    })
}

/// Refuse an improve pass whose estimate would push the tenant past its
/// teacher-GPU budget. Same cap as extraction: both spend our own metered GPU on
/// running somebody's teacher.
pub async fn admit_on_policy(
    billing_repo: &dyn BillingEventRepository,
    tenant_id: Uuid,
    cap: Option<f64>,
    plan: &OnPolicyPlan,
) -> AppResult<()> {
    check_teacher_gpu_spend_cap(billing_repo, tenant_id, cap, plan.estimate.est_cost_usd).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    const PARENT_ADAPTER: &str = "tenants/t/models/parent/";

    fn rate(_class: &str) -> f64 {
        6.00
    }

    fn dataset(teacher_model: &str, pairs: Option<i32>) -> Dataset {
        Dataset {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            name: "d".to_string(),
            storage_path: None,
            format: "chatml".to_string(),
            status: "approved".to_string(),
            pair_count: pairs,
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
            scored_completion_tokens: None,
            token_count_tokenizer_hash: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn only_the_on_policy_method_asks_for_an_improve_pass() {
        assert!(wants_on_policy(&DistillOptionsDto {
            method: Some(DistillMethod::OnPolicy),
            ..Default::default()
        }));
        assert!(!wants_on_policy(&DistillOptionsDto {
            method: Some(DistillMethod::Logit),
            ..Default::default()
        }));
        assert!(!wants_on_policy(&DistillOptionsDto::default()));
    }

    /// Both fidelity paths write the same `teacher_config` key, so a job that
    /// somehow asked for both would have one plan silently overwrite the other.
    /// The method enum is what makes that unrepresentable.
    #[test]
    fn no_option_set_asks_for_both_fidelity_paths_at_once() {
        for method in [
            None,
            Some(DistillMethod::Text),
            Some(DistillMethod::Logit),
            Some(DistillMethod::OnPolicy),
        ] {
            let options = DistillOptionsDto {
                method,
                ..Default::default()
            };
            assert!(!(options.wants_logits() && wants_on_policy(&options)));
        }
    }

    /// The teacher needs a card to itself and the student another, so the class is
    /// always a paired one — never the single-card class the teacher alone fits on.
    #[test]
    fn every_paired_class_has_two_devices() {
        for teacher_class in [
            GpuClass::T4,
            GpuClass::A10g,
            GpuClass::L40s,
            GpuClass::A10040gb,
            GpuClass::A10080gb,
            GpuClass::H100,
        ] {
            assert_eq!(paired_gpu_class(teacher_class).device_count(), 2);
        }
    }

    #[test]
    fn an_h100_teacher_keeps_h100_class_hardware() {
        assert_eq!(paired_gpu_class(GpuClass::H100), GpuClass::H100Dual);
    }

    #[test]
    fn rollout_tokens_scale_with_examples_and_epochs() {
        let (one_epoch, _) = rollout_tokens_for(Some(100), 1);
        let (three_epochs, _) = rollout_tokens_for(Some(100), 3);

        assert_eq!(three_epochs, one_epoch * 3);
    }

    /// The student's answers do not exist yet, so no basis but approximate is
    /// honest — and the UI says so rather than showing a measured-looking number.
    #[test]
    fn rollout_tokens_are_never_reported_as_measured() {
        let (_, basis) = rollout_tokens_for(Some(1000), 1);

        assert_eq!(basis, EstimateBasis::Approximate);
    }

    #[test]
    fn a_missing_pair_count_does_not_panic_or_go_negative() {
        assert_eq!(rollout_tokens_for(None, 1).0, 0);
        assert_eq!(rollout_tokens_for(Some(-5), 1).0, 0);
    }

    #[test]
    fn zero_epochs_still_prices_one_pass() {
        assert_eq!(
            rollout_tokens_for(Some(10), 0),
            rollout_tokens_for(Some(10), 1)
        );
    }

    /// The quote is per epoch of rollouts, so quoting a different number from the
    /// one the job will train is a wrong bill in whichever direction they differ.
    /// The worker's own default is asserted against this figure in
    /// `tests/test_on_policy.py::test_the_epoch_default_matches_what_the_api_quotes`.
    #[test]
    fn an_unspecified_epoch_count_is_priced_at_what_the_job_will_get() {
        assert_eq!(DEFAULT_EPOCHS, 3);
    }

    /// The plan is what the worker trains from, so the number it carries has to be
    /// the number the estimate was computed from — not a default resolved twice.
    /// The worker reads this to decide whether to continue the parent's adapter or
    /// attach a fresh one, so a plan that loses it trains from scratch in silence.
    #[test]
    fn the_plan_carries_the_adapter_the_run_continues_from() {
        let plan = plan_on_policy(
            &dataset("Qwen/Qwen3-32B", Some(100)),
            "Qwen/Qwen3-8B",
            None,
            PARENT_ADAPTER,
            40.0,
            rate,
        )
        .expect("a hosted teacher is plannable");

        let block = attach_to_teacher_config(Some(json!({"model": "t"})), Some(&plan))
            .expect("a block to live on")
            .expect("a block");

        assert_eq!(block["extraction"]["parent_adapter_path"], PARENT_ADAPTER);
    }

    /// Fail closed: by this point the tenant has been quoted and admitted for a
    /// two-card improve pass, so a run that trained from scratch would charge them
    /// for continuing a model it never loaded.
    #[test]
    fn a_plan_with_no_parent_to_continue_is_an_error_not_a_fresh_run() {
        let plan = plan_on_policy(
            &dataset("Qwen/Qwen3-32B", Some(100)),
            "Qwen/Qwen3-8B",
            None,
            "",
            40.0,
            rate,
        )
        .expect("a hosted teacher is plannable");

        attach_to_teacher_config(Some(json!({"model": "t"})), Some(&plan))
            .expect_err("an improve pass with nothing to improve must not be persisted");
    }

    #[test]
    fn the_plan_carries_the_epoch_count_its_estimate_was_priced_from() {
        let quoted = plan_on_policy(
            &dataset("Qwen/Qwen3-32B", Some(100)),
            "Qwen/Qwen3-8B",
            Some(2),
            PARENT_ADAPTER,
            50.0,
            rate,
        )
        .unwrap();
        let defaulted = plan_on_policy(
            &dataset("Qwen/Qwen3-32B", Some(100)),
            "Qwen/Qwen3-8B",
            None,
            PARENT_ADAPTER,
            50.0,
            rate,
        )
        .unwrap();

        assert_eq!(quoted.epochs, 2);
        assert_eq!(quoted.workflow_value()["epochs"], 2);
        assert_eq!(defaulted.epochs, DEFAULT_EPOCHS);
        assert_eq!(
            defaulted.estimate.scored_tokens,
            quoted.estimate.scored_tokens / 2 * DEFAULT_EPOCHS
        );
    }

    #[test]
    fn a_nonsensical_epoch_count_is_priced_and_recorded_as_one_pass() {
        let plan = plan_on_policy(
            &dataset("Qwen/Qwen3-32B", Some(10)),
            "Qwen/Qwen3-8B",
            Some(0),
            PARENT_ADAPTER,
            50.0,
            rate,
        )
        .unwrap();

        assert_eq!(plan.epochs, 1);
    }

    #[test]
    fn a_plan_names_the_catalog_teacher_and_a_paired_class() {
        let plan = plan_on_policy(
            &dataset("Qwen/Qwen3-32B", Some(500)),
            "Qwen/Qwen3-8B",
            None,
            PARENT_ADAPTER,
            50.0,
            rate,
        )
        .unwrap();

        assert_eq!(plan.teacher_model, "Qwen/Qwen3-32B");
        assert!(!plan.teacher_revision.is_empty());
        assert_eq!(plan.gpu_class, GpuClass::A10080gbDual.to_string());
        assert_eq!(plan.precision, TeacherPrecision::Bf16);
    }

    #[test]
    fn a_dataset_whose_teacher_we_cannot_run_is_refused() {
        let refusal = plan_on_policy(
            &dataset("some-closed-model", Some(500)),
            "Qwen/Qwen3-8B",
            None,
            PARENT_ADAPTER,
            50.0,
            rate,
        )
        .unwrap_err();

        assert!(!refusal.is_empty());
    }

    #[test]
    fn the_block_says_which_method_the_workflow_should_dispatch() {
        let plan = plan_on_policy(
            &dataset("Qwen/Qwen3-32B", Some(10)),
            "Qwen/Qwen3-8B",
            None,
            PARENT_ADAPTER,
            50.0,
            rate,
        )
        .unwrap();
        let block = plan.workflow_value();

        assert_eq!(block["distill_method"], "on_policy");
        assert_eq!(block["teacher_model"], "Qwen/Qwen3-32B");
        assert_eq!(block["gpu_class"], GpuClass::A10080gbDual.to_string());
    }

    #[test]
    fn attaching_a_plan_preserves_the_teacher_it_merges_into() {
        let plan = plan_on_policy(
            &dataset("Qwen/Qwen3-32B", Some(10)),
            "Qwen/Qwen3-8B",
            None,
            PARENT_ADAPTER,
            50.0,
            rate,
        )
        .unwrap();
        let teacher = json!({"host": "inference.example.com", "model": "Qwen/Qwen3-32B"});

        let merged = attach_to_teacher_config(Some(teacher), Some(&plan))
            .unwrap()
            .unwrap();

        assert_eq!(merged["host"], "inference.example.com");
        assert_eq!(merged["extraction"]["distill_method"], "on_policy");
    }

    #[test]
    fn no_plan_leaves_the_teacher_block_untouched() {
        let teacher = json!({"model": "Qwen/Qwen3-32B"});

        assert_eq!(
            attach_to_teacher_config(Some(teacher.clone()), None)
                .unwrap()
                .unwrap(),
            teacher
        );
    }

    #[test]
    fn a_job_without_a_teacher_stays_without_one() {
        assert!(attach_to_teacher_config(None, None).unwrap().is_none());
    }

    /// By this point the tenant has been quoted a two-card rate and admitted against
    /// their teacher budget. A job that continued without the plan would train plain
    /// SFT on hardware it is being charged a teacher's price for, and nothing would
    /// say so — the whole point of the guard upstream, kept here as well because the
    /// cost of being wrong is a wrong bill.
    #[test]
    fn an_admitted_plan_with_nowhere_to_live_is_an_error_not_a_dropped_plan() {
        let plan = plan_on_policy(
            &dataset("Qwen/Qwen3-32B", Some(10)),
            "Qwen/Qwen3-8B",
            None,
            PARENT_ADAPTER,
            50.0,
            rate,
        )
        .unwrap();

        assert!(attach_to_teacher_config(None, Some(&plan)).is_err());
    }
}
