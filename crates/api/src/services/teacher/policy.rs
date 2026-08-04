//! Provider-terms classification for teacher endpoints.
//!
//! Distillation trains on a teacher's outputs, and some API providers'
//! terms restrict that. The platform does not interpret anyone's terms — it
//! classifies the teacher into a coarse policy and lets the user decide:
//! `Restricted` requires an explicit acknowledgment before a run starts,
//! `Unknown` is informational, `Allowed` is reserved for catalog models
//! whose weights ship under permissive licenses.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Hosts of proprietary-model APIs whose terms of service restrict using outputs
/// to train other models. Hostname match only — path and port are irrelevant to
/// whose terms apply.
///
/// Defaults only. Providers revise their terms, and a list compiled into the
/// binary would mean a release every time one does, so an operator replaces this
/// via [`init_restricted_hosts`].
const DEFAULT_RESTRICTED_HOSTS: &[&str] = &[
    "api.openai.com",
    "api.anthropic.com",
    "generativelanguage.googleapis.com",
    "api.x.ai",
    "api.cohere.com",
    "api.mistral.ai",
];

static RESTRICTED_HOSTS: OnceLock<Vec<String>> = OnceLock::new();

/// Install the operator's restricted-host list, replacing the defaults.
///
/// Hosts are lowercased on the way in so classification stays a plain
/// comparison. Returns the previous state as an error if already initialized;
/// call once at startup.
pub fn init_restricted_hosts(hosts: Vec<String>) -> Result<(), ()> {
    RESTRICTED_HOSTS
        .set(
            hosts
                .into_iter()
                .map(|host| host.trim().to_ascii_lowercase())
                .filter(|host| !host.is_empty())
                .collect(),
        )
        .map_err(|_| ())
}

/// Hosts currently treated as restricted.
///
/// An operator may legitimately configure an empty list — that means "classify
/// nothing as restricted here", not "fall back to the defaults", so emptiness is
/// respected rather than second-guessed.
pub fn restricted_hosts() -> &'static [String] {
    RESTRICTED_HOSTS.get_or_init(|| {
        DEFAULT_RESTRICTED_HOSTS
            .iter()
            .map(|host| host.to_string())
            .collect()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProviderPolicy {
    Allowed,
    Restricted,
    Unknown,
}

impl ProviderPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderPolicy::Allowed => "allowed",
            ProviderPolicy::Restricted => "restricted",
            ProviderPolicy::Unknown => "unknown",
        }
    }

    /// Provenance blocks store the policy as a string; unrecognized values
    /// (older data, manual edits) read back as `Unknown`, never an error.
    pub fn parse_lossy(value: &str) -> Self {
        match value {
            "allowed" => ProviderPolicy::Allowed,
            "restricted" => ProviderPolicy::Restricted,
            _ => ProviderPolicy::Unknown,
        }
    }
}

/// A curated teacher model with a permissive weights license, safe to
/// distill from wherever it is hosted.
#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct TeacherCatalogEntry {
    pub model_id: &'static str,
    pub license: &'static str,
    /// One-line "why this one" shown in the teacher picker.
    pub why: &'static str,
}

/// Recommended open teacher models. Entries must ship weights under a
/// permissive license (Apache-2.0 / MIT) — that license, not the hosting
/// provider, is what makes training on their outputs unambiguous.
pub fn teacher_catalog() -> &'static [TeacherCatalogEntry] {
    &[
        TeacherCatalogEntry {
            model_id: "Qwen/Qwen3-32B",
            license: "Apache-2.0",
            why: "Strong general-purpose teacher with a permissive license",
        },
        TeacherCatalogEntry {
            model_id: "Qwen/Qwen2.5-32B-Instruct",
            license: "Apache-2.0",
            why: "Reliable instruction-following teacher, widely hosted",
        },
        TeacherCatalogEntry {
            model_id: "deepseek-ai/DeepSeek-R1",
            license: "MIT",
            why: "Strongest open reasoning teacher; exposes full reasoning traces",
        },
        TeacherCatalogEntry {
            model_id: "mistralai/Mixtral-8x22B-Instruct-v0.1",
            license: "Apache-2.0",
            why: "Large mixture-of-experts teacher for broad general tasks",
        },
        TeacherCatalogEntry {
            model_id: "microsoft/phi-4",
            license: "MIT",
            why: "Compact teacher that keeps generation costs low",
        },
    ]
}

/// Classify a teacher by endpoint host and model id.
///
/// Restricted host wins over a catalog model: routing an open model id
/// through a proprietary API host still puts that host's terms in play.
pub fn classify_provider(api_base_url: &str, model: &str) -> ProviderPolicy {
    if let Some(host) = host_of(api_base_url) {
        let host = host.to_ascii_lowercase();
        if restricted_hosts().iter().any(|known| known == &host) {
            return ProviderPolicy::Restricted;
        }
    }
    let model_lower = model.to_ascii_lowercase();
    if teacher_catalog()
        .iter()
        .any(|entry| entry.model_id.to_ascii_lowercase() == model_lower)
    {
        return ProviderPolicy::Allowed;
    }
    ProviderPolicy::Unknown
}

/// Displayable host of a teacher endpoint (no scheme, path, or credentials).
pub fn host_of(api_base_url: &str) -> Option<String> {
    reqwest::Url::parse(api_base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_hosts_classified() {
        assert_eq!(
            classify_provider("https://api.openai.com/v1", "gpt-anything"),
            ProviderPolicy::Restricted
        );
        assert_eq!(
            classify_provider("https://API.OpenAI.com/v1", "gpt-anything"),
            ProviderPolicy::Restricted
        );
    }

    #[test]
    fn restricted_host_wins_over_catalog_model() {
        assert_eq!(
            classify_provider("https://api.anthropic.com/v1", "Qwen/Qwen3-32B"),
            ProviderPolicy::Restricted
        );
    }

    #[test]
    fn catalog_model_allowed_on_any_public_host() {
        assert_eq!(
            classify_provider("https://inference.example.com/v1", "Qwen/Qwen3-32B"),
            ProviderPolicy::Allowed
        );
        assert_eq!(
            classify_provider("https://inference.example.com/v1", "qwen/qwen3-32b"),
            ProviderPolicy::Allowed
        );
    }

    #[test]
    fn everything_else_unknown() {
        assert_eq!(
            classify_provider("https://inference.example.com/v1", "some-model"),
            ProviderPolicy::Unknown
        );
        assert_eq!(
            classify_provider("not a url", "some-model"),
            ProviderPolicy::Unknown
        );
    }

    #[test]
    fn policy_string_roundtrip() {
        for policy in [
            ProviderPolicy::Allowed,
            ProviderPolicy::Restricted,
            ProviderPolicy::Unknown,
        ] {
            assert_eq!(ProviderPolicy::parse_lossy(policy.as_str()), policy);
        }
        assert_eq!(
            ProviderPolicy::parse_lossy("garbage"),
            ProviderPolicy::Unknown
        );
    }
}
