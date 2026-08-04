//! Catalog of teachers the platform runs itself.
//!
//! Logit distillation needs the teacher's per-token probability distributions,
//! which no chat API exposes — the teacher has to run on a GPU we control. That
//! makes a hosted teacher a very different security proposition from an endpoint
//! the tenant points us at: we execute these weights, so the runnable set is an
//! allowlist rather than anything a request can name.
//!
//! The allowlist is **operator-configured data, not source code**. Open-weight
//! model families turn over in months, so adding tomorrow's teacher must be a
//! deployment change, never a release: [`init_hosted_catalog`] replaces the
//! built-in defaults from whatever the operator supplies at startup. What stays
//! closed is *who* chooses — a tenant request can only ask for a teacher already
//! on the list.
//!
//! Every entry carries a **pinned revision**, because a moved tag would
//! otherwise silently change what we execute, and models are always loaded with
//! `trust_remote_code` off so a repository cannot ship code onto our GPUs.

use std::sync::OnceLock;

use platform_shared::enums::GpuClass;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// A teacher the platform runs on its own GPUs.
#[derive(Debug, Clone, Deserialize, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct HostedTeacherEntry {
    pub model_id: String,
    /// Commit the weights are pinned to. Never a branch or tag.
    pub revision: String,
    pub license: String,
    pub display_name: String,
    /// Shown in the picker alongside the name.
    pub size: String,
    /// GPU class this model is scheduled on at its default precision.
    pub gpu_class: GpuClass,
    /// Tokens per second the extraction job is expected to score, at this
    /// entry's default precision. Deliberately conservative: scoring computes a
    /// distribution at *every* position, which is far heavier than ordinary
    /// prefill, and an estimate that comes in under the real bill is worse than
    /// one that comes in over it. Real runs bill measured time, not this.
    pub est_scored_tokens_per_sec: f64,
    /// Student models whose tokenizer is expected to match this teacher's.
    /// Advisory only — eligibility is decided by the tokenizer hash comparison
    /// in the workers, never by a family name in this list.
    ///
    /// Upstream repositories only. Community re-uploads of the same weights are
    /// measurably *not* interchangeable here: `unsloth/Qwen2.5-7B-Instruct` adds
    /// a `special_tokens_map.json` declaring a pad token that upstream Qwen does
    /// not ship, so it fails the hash comparison for a real reason.
    pub student_family: Vec<String>,
}

/// Why an operator-supplied catalog was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("hosted teacher catalog is empty")]
    Empty,
    #[error("{model_id} must pin a 40-character commit sha, not a branch or tag")]
    UnpinnedRevision { model_id: String },
    #[error("{model_id} lists no student it could teach")]
    NoStudent { model_id: String },
    #[error("{model_id} has a non-positive throughput estimate")]
    InvalidThroughput { model_id: String },
    #[error("{model_id} appears more than once")]
    Duplicate { model_id: String },
    #[error("hosted teacher catalog is already initialized")]
    AlreadyInitialized,
}

static HOSTED_CATALOG: OnceLock<Vec<HostedTeacherEntry>> = OnceLock::new();

/// Install an operator-supplied catalog, replacing the built-in defaults.
///
/// Rejected wholesale on any invalid entry rather than filtering: a catalog that
/// silently drops the teacher an operator meant to add is worse than one that
/// refuses to start and says why. Callable once, at startup, before any request
/// has read the catalog.
pub fn init_hosted_catalog(entries: Vec<HostedTeacherEntry>) -> Result<(), CatalogError> {
    validate_catalog(&entries)?;
    HOSTED_CATALOG
        .set(entries)
        .map_err(|_| CatalogError::AlreadyInitialized)
}

/// Teachers available for hosted extraction.
pub fn hosted_catalog() -> &'static [HostedTeacherEntry] {
    HOSTED_CATALOG.get_or_init(default_hosted_catalog)
}

/// Look up a hosted entry by model id. A teacher naming anything else is
/// rejected rather than attempted — this lookup *is* the allowlist.
pub fn hosted_entry(model_id: &str) -> Option<&'static HostedTeacherEntry> {
    hosted_catalog()
        .iter()
        .find(|entry| entry.model_id.eq_ignore_ascii_case(model_id.trim()))
}

fn validate_catalog(entries: &[HostedTeacherEntry]) -> Result<(), CatalogError> {
    if entries.is_empty() {
        return Err(CatalogError::Empty);
    }
    for (index, entry) in entries.iter().enumerate() {
        if entry.revision.len() != 40 || !entry.revision.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CatalogError::UnpinnedRevision {
                model_id: entry.model_id.clone(),
            });
        }
        if entry.student_family.is_empty() {
            return Err(CatalogError::NoStudent {
                model_id: entry.model_id.clone(),
            });
        }
        let throughput = entry.est_scored_tokens_per_sec;
        if !throughput.is_finite() || throughput <= 0.0 {
            return Err(CatalogError::InvalidThroughput {
                model_id: entry.model_id.clone(),
            });
        }
        if entries[..index]
            .iter()
            .any(|earlier| earlier.model_id.eq_ignore_ascii_case(&entry.model_id))
        {
            return Err(CatalogError::Duplicate {
                model_id: entry.model_id.clone(),
            });
        }
    }
    Ok(())
}

/// Built-in defaults, used when an operator supplies nothing.
///
/// Deliberately narrow: an entry only earns its place if a student in the
/// base-model catalog can plausibly share its tokenizer, because logit
/// distillation is impossible otherwise. Dense models only — mixture-of-experts
/// teachers add routing nondeterminism to a pass whose entire product is
/// reproducible probabilities, and no measurement was found either way.
fn default_hosted_catalog() -> Vec<HostedTeacherEntry> {
    vec![
        HostedTeacherEntry {
            model_id: "Qwen/Qwen3-32B".to_string(),
            revision: "9216db5781bf21249d130ec9da846c4624c16137".to_string(),
            license: "Apache-2.0".to_string(),
            display_name: "Qwen3 32B".to_string(),
            size: "32B".to_string(),
            gpu_class: GpuClass::A10080gb,
            est_scored_tokens_per_sec: 1200.0,
            student_family: vec![
                "Qwen/Qwen3-8B".to_string(),
                "Qwen/Qwen3-4B".to_string(),
                "Qwen/Qwen3-14B".to_string(),
            ],
        },
        HostedTeacherEntry {
            model_id: "Qwen/Qwen2.5-32B-Instruct".to_string(),
            revision: "5ede1c97bbab6ce5cda5812749b4c0bdf79b18dd".to_string(),
            license: "Apache-2.0".to_string(),
            display_name: "Qwen2.5 32B Instruct".to_string(),
            size: "32B".to_string(),
            gpu_class: GpuClass::A10080gb,
            est_scored_tokens_per_sec: 1200.0,
            student_family: vec!["Qwen/Qwen2.5-7B-Instruct".to_string()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(model_id: &str) -> HostedTeacherEntry {
        HostedTeacherEntry {
            model_id: model_id.to_string(),
            revision: "0".repeat(40),
            license: "Apache-2.0".to_string(),
            display_name: "T".to_string(),
            size: "32B".to_string(),
            gpu_class: GpuClass::A10080gb,
            est_scored_tokens_per_sec: 1000.0,
            student_family: vec!["S".to_string()],
        }
    }

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
    fn defaults_satisfy_the_same_rules_an_operator_catalog_must() {
        validate_catalog(&default_hosted_catalog()).expect("shipped defaults must be valid");
    }

    #[test]
    fn an_empty_catalog_is_refused() {
        assert_eq!(validate_catalog(&[]), Err(CatalogError::Empty));
    }

    #[test]
    fn a_branch_or_tag_instead_of_a_commit_is_refused() {
        let mut candidate = entry("org/model");
        candidate.revision = "main".to_string();

        assert_eq!(
            validate_catalog(&[candidate]),
            Err(CatalogError::UnpinnedRevision {
                model_id: "org/model".to_string()
            })
        );
    }

    #[test]
    fn a_revision_of_the_right_length_but_not_hex_is_refused() {
        let mut candidate = entry("org/model");
        candidate.revision = "z".repeat(40);

        assert!(matches!(
            validate_catalog(&[candidate]),
            Err(CatalogError::UnpinnedRevision { .. })
        ));
    }

    #[test]
    fn a_teacher_with_no_student_is_refused() {
        let mut candidate = entry("org/model");
        candidate.student_family.clear();

        assert_eq!(
            validate_catalog(&[candidate]),
            Err(CatalogError::NoStudent {
                model_id: "org/model".to_string()
            })
        );
    }

    #[test]
    fn a_nonpositive_throughput_is_refused() {
        for throughput in [0.0, -1.0, f64::NAN] {
            let mut candidate = entry("org/model");
            candidate.est_scored_tokens_per_sec = throughput;

            assert!(matches!(
                validate_catalog(&[candidate]),
                Err(CatalogError::InvalidThroughput { .. })
            ));
        }
    }

    #[test]
    fn a_duplicate_model_id_is_refused() {
        assert_eq!(
            validate_catalog(&[entry("org/model"), entry("ORG/MODEL")]),
            Err(CatalogError::Duplicate {
                model_id: "ORG/MODEL".to_string()
            })
        );
    }

    #[test]
    fn an_operator_catalog_parses_from_json() {
        let parsed: Vec<HostedTeacherEntry> = serde_json::from_str(
            r#"[{
                "model_id": "org/next-years-model",
                "revision": "1111111111111111111111111111111111111111",
                "license": "Apache-2.0",
                "display_name": "Next Year 40B",
                "size": "40B",
                "gpu_class": "h100",
                "est_scored_tokens_per_sec": 900.0,
                "student_family": ["org/next-years-model-4b"]
            }]"#,
        )
        .expect("operator catalog should parse");

        validate_catalog(&parsed).expect("valid entry");
        assert_eq!(parsed[0].gpu_class, GpuClass::H100);
    }
}
