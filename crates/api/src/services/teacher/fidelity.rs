//! Whether a dataset can be trained on its teacher's token-level confidence.
//!
//! Logit distillation needs the teacher's per-token probability distributions,
//! which no chat API returns — so the teacher has to be one we can run
//! ourselves. Rather than asking the user to pick a second teacher, the scorer
//! is *derived* from the dataset's own provenance: if the model that wrote this
//! data is in the hosted catalog, we can run that same model to score it, and
//! the text and the distributions then come from one teacher instead of two.
//!
//! Everything here is advisory and cheap enough to answer on a page load. The
//! authoritative compatibility check is the tokenizer-artifact hash comparison
//! performed by the extraction workflow, which is the only thing that can prove
//! two models read text identically.

use serde_json::Value;

use crate::services::teacher::config::provenance_from_config;
use crate::services::teacher::hosted::{HostedTeacherEntry, hosted_entry};

/// How much of the teacher's distribution is kept per token.
///
/// Published results put the useful range near k=5–10 with clear diminishing
/// returns above it (token distributions are Zipfian, so the head carries almost
/// all the mass), while cost grows with k on three axes at once: artifact size,
/// scoring memory, and vLLM's roughly linear slowdown as the requested logprob
/// count rises. 32 sits comfortably above the reported optimum without paying
/// four times over for mass that contributes almost nothing.
pub const DEFAULT_TOP_K_LOGPROBS: u32 = 32;

/// Bounds on a caller-supplied `top_k`. An uncapped request is an out-of-memory
/// crash on the scoring GPU, so there is no "unlimited" option to choose.
pub const MIN_TOP_K_LOGPROBS: u32 = 8;
pub const MAX_TOP_K_LOGPROBS: u32 = 256;

/// Why a dataset cannot be trained at higher fidelity.
///
/// These reach the user verbatim, so each one says what is wrong *and* what
/// still works — an offer that silently disappears is worse than one that
/// explains itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FidelityBlocker {
    NotTeacherGenerated,
    TeacherNotSelfHostable,
    StudentReadsTextDifferently,
}

impl FidelityBlocker {
    pub fn message(self) -> &'static str {
        match self {
            FidelityBlocker::NotTeacherGenerated => {
                "This data was not written by a teacher, so there is no teacher confidence to \
                 learn from. Generate with a teacher first."
            }
            FidelityBlocker::TeacherNotSelfHostable => {
                "Higher-fidelity training needs a teacher we can run ourselves, and this one is \
                 only reachable as an API. Standard distillation is unaffected."
            }
            FidelityBlocker::StudentReadsTextDifferently => {
                "These two models read text differently, so high-fidelity training is not \
                 possible between them. Standard distillation works — switch and re-run."
            }
        }
    }
}

/// Clamp a caller-supplied `top_k` into the supported range.
pub fn clamp_top_k(requested: Option<u32>) -> u32 {
    requested
        .unwrap_or(DEFAULT_TOP_K_LOGPROBS)
        .clamp(MIN_TOP_K_LOGPROBS, MAX_TOP_K_LOGPROBS)
}

/// Find the hosted teacher that could score `dataset_config`'s data for
/// `student_model`, or the reason none can.
pub fn hosted_scorer_for(
    dataset_config: &Value,
    student_model: &str,
) -> Result<&'static HostedTeacherEntry, FidelityBlocker> {
    let provenance =
        provenance_from_config(dataset_config).ok_or(FidelityBlocker::NotTeacherGenerated)?;
    let entry = hosted_entry(&provenance.model).ok_or(FidelityBlocker::TeacherNotSelfHostable)?;
    let student = student_model.trim();
    if entry
        .student_family
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(student))
    {
        Ok(entry)
    } else {
        Err(FidelityBlocker::StudentReadsTextDifferently)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dataset_with_teacher(model: &str) -> Value {
        json!({"teacher": {
            "host": "inference.example.com",
            "model": model,
            "policy": "allowed",
            "cot": false,
            "generated_at": "2026-08-04T00:00:00Z",
        }})
    }

    #[test]
    fn hosted_teacher_and_matching_student_is_eligible() {
        let entry = hosted_scorer_for(
            &dataset_with_teacher("Qwen/Qwen2.5-32B-Instruct"),
            "Qwen/Qwen2.5-7B-Instruct",
        )
        .expect("should be eligible");
        assert_eq!(entry.model_id, "Qwen/Qwen2.5-32B-Instruct");
    }

    #[test]
    fn dataset_without_teacher_is_blocked() {
        assert_eq!(
            hosted_scorer_for(&json!({}), "Qwen/Qwen2.5-7B-Instruct").unwrap_err(),
            FidelityBlocker::NotTeacherGenerated
        );
    }

    #[test]
    fn api_only_teacher_is_blocked() {
        assert_eq!(
            hosted_scorer_for(
                &dataset_with_teacher("some-proprietary-model"),
                "Qwen/Qwen2.5-7B-Instruct"
            )
            .unwrap_err(),
            FidelityBlocker::TeacherNotSelfHostable
        );
    }

    #[test]
    fn student_from_another_family_is_blocked() {
        assert_eq!(
            hosted_scorer_for(
                &dataset_with_teacher("Qwen/Qwen2.5-32B-Instruct"),
                "unsloth/Llama-3.2-1B-Instruct"
            )
            .unwrap_err(),
            FidelityBlocker::StudentReadsTextDifferently
        );
    }

    #[test]
    fn every_blocker_says_what_still_works() {
        for blocker in [
            FidelityBlocker::NotTeacherGenerated,
            FidelityBlocker::TeacherNotSelfHostable,
            FidelityBlocker::StudentReadsTextDifferently,
        ] {
            let message = blocker.message();
            assert!(message.len() > 40, "message is too terse to be useful");
            assert!(!message.contains("logit") && !message.contains("logprob"));
        }
    }

    #[test]
    fn top_k_is_clamped_into_the_supported_range() {
        assert_eq!(clamp_top_k(None), DEFAULT_TOP_K_LOGPROBS);
        assert_eq!(clamp_top_k(Some(0)), MIN_TOP_K_LOGPROBS);
        assert_eq!(clamp_top_k(Some(1_000_000)), MAX_TOP_K_LOGPROBS);
        assert_eq!(clamp_top_k(Some(64)), 64);
    }
}
