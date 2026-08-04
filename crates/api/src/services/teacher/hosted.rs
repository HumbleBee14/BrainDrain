//! Catalog of teachers the platform runs itself.
//!
//! Logit distillation needs the teacher's per-token probability distributions,
//! which no chat API exposes — the teacher has to run on a GPU we control. That
//! makes a hosted teacher a very different security proposition from an
//! endpoint the tenant points us at: we execute these weights, so the set of
//! runnable models is a closed, reviewed catalog rather than anything a request
//! can name.
//!
//! Every entry therefore carries a **pinned revision** (a moved tag must never
//! silently change what we execute) and is loaded with `trust_remote_code`
//! off, so a repository cannot ship code that runs on our GPUs.

use platform_shared::enums::GpuClass;
use serde::Serialize;
use ts_rs::TS;
use utoipa::ToSchema;

/// A teacher the platform runs on its own GPUs.
#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct HostedTeacherEntry {
    pub model_id: &'static str,
    /// Commit the weights are pinned to. Never a branch or tag.
    pub revision: &'static str,
    pub license: &'static str,
    pub display_name: &'static str,
    /// Shown in the picker alongside the name.
    pub size: &'static str,
    /// GPU class this model is scheduled on at its default precision.
    pub gpu_class: GpuClass,
    /// Tokens per second the extraction job is expected to score. Deliberately
    /// conservative: scoring computes a distribution at *every* position, which
    /// is far heavier than ordinary prefill, and an estimate that comes in under
    /// the real bill is worse than one that comes in over it.
    pub est_scored_tokens_per_sec: f64,
    /// Student models whose tokenizer is expected to match this teacher's.
    /// Advisory only — eligibility is decided by the tokenizer hash comparison
    /// in the workers, never by a family name in this list.
    ///
    /// Upstream repositories only. Community re-uploads of the same weights are
    /// measurably *not* interchangeable here: `unsloth/Qwen2.5-7B-Instruct` adds
    /// a `special_tokens_map.json` declaring a pad token that upstream Qwen does
    /// not ship, so it fails the hash comparison for a real reason.
    pub student_family: &'static [&'static str],
}

/// Teachers available for hosted extraction.
///
/// Launch set is deliberately narrow: an entry only earns its place if we have
/// a student in the base-model catalog that can plausibly share its tokenizer,
/// because logit distillation is impossible otherwise. Adding a large teacher
/// with no matching student would ship a feature that can never be eligible.
pub fn hosted_catalog() -> &'static [HostedTeacherEntry] {
    &[
        HostedTeacherEntry {
            model_id: "Qwen/Qwen2.5-32B-Instruct",
            revision: "5ede1c97bbab6ce5cda5812749b4c0bdf79b18dd",
            license: "Apache-2.0",
            display_name: "Qwen2.5 32B Instruct",
            size: "32B",
            gpu_class: GpuClass::A10080gb,
            est_scored_tokens_per_sec: 1500.0,
            student_family: &["Qwen/Qwen2.5-7B-Instruct"],
        },
        HostedTeacherEntry {
            model_id: "Qwen/Qwen2.5-14B-Instruct",
            revision: "cf98f3b3bbb457ad9e2bb7baf9a0125b6b88caa8",
            license: "Apache-2.0",
            display_name: "Qwen2.5 14B Instruct",
            size: "14B",
            gpu_class: GpuClass::L40s,
            est_scored_tokens_per_sec: 3000.0,
            student_family: &["Qwen/Qwen2.5-7B-Instruct"],
        },
    ]
}

/// Look up a hosted entry by model id. A teacher naming anything else is
/// rejected rather than attempted — this lookup *is* the allowlist.
pub fn hosted_entry(model_id: &str) -> Option<&'static HostedTeacherEntry> {
    hosted_catalog()
        .iter()
        .find(|entry| entry.model_id.eq_ignore_ascii_case(model_id.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_is_case_insensitive_and_trims() {
        assert!(hosted_entry("qwen/qwen2.5-32b-instruct").is_some());
        assert!(hosted_entry("  Qwen/Qwen2.5-32B-Instruct  ").is_some());
    }

    #[test]
    fn unknown_model_is_not_in_the_allowlist() {
        assert!(hosted_entry("evil/backdoored-model").is_none());
        assert!(hosted_entry("").is_none());
    }

    #[test]
    fn every_entry_pins_a_commit_not_a_tag() {
        for entry in hosted_catalog() {
            assert_eq!(
                entry.revision.len(),
                40,
                "{} must pin a full 40-char commit sha",
                entry.model_id
            );
            assert!(
                entry.revision.chars().all(|c| c.is_ascii_hexdigit()),
                "{} revision must be hex",
                entry.model_id
            );
        }
    }

    #[test]
    fn every_entry_names_a_plausible_student() {
        for entry in hosted_catalog() {
            assert!(
                !entry.student_family.is_empty(),
                "{} has no student it could ever teach",
                entry.model_id
            );
        }
    }

    #[test]
    fn throughput_estimates_are_positive() {
        for entry in hosted_catalog() {
            assert!(entry.est_scored_tokens_per_sec > 0.0);
        }
    }
}
