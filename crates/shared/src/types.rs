//! Typed structs for JSON blob fields used in DB models and API responses.
//!
//! These replace raw `serde_json::Value` with proper types, giving us
//! compile-time safety and documentation of the expected JSON structure.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;
use utoipa::ToSchema;

/// Training hyperparameters (stored in training_jobs.hyperparams).
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS, ToSchema)]
#[ts(export)]
pub struct Hyperparams {
    #[serde(default = "default_r")]
    pub r: i32,
    #[serde(default = "default_lora_alpha")]
    pub lora_alpha: i32,
    #[serde(default)]
    pub lora_dropout: i32,
    #[serde(default = "default_target_modules")]
    pub target_modules: Vec<String>,
    #[serde(default = "default_learning_rate")]
    pub learning_rate: f64,
    #[serde(default = "default_batch_size")]
    pub per_device_train_batch_size: i32,
    #[serde(default = "default_grad_accum")]
    pub gradient_accumulation_steps: i32,
    #[serde(default = "default_epochs")]
    pub num_train_epochs: i32,
    #[serde(default = "default_warmup")]
    pub warmup_steps: i32,
    #[serde(default = "default_optim")]
    pub optim: String,
    #[serde(default = "default_scheduler")]
    pub lr_scheduler_type: String,
    #[serde(default = "default_seq_length")]
    pub max_seq_length: i32,
    /// Extra user-provided params not in the schema.
    #[serde(flatten)]
    #[ts(flatten)]
    #[schema(value_type = Object)]
    pub extra: HashMap<String, serde_json::Value>,
}

fn default_r() -> i32 {
    16
}
fn default_lora_alpha() -> i32 {
    16
}
fn default_target_modules() -> Vec<String> {
    vec![
        "q_proj".into(),
        "k_proj".into(),
        "v_proj".into(),
        "o_proj".into(),
        "gate_proj".into(),
        "up_proj".into(),
        "down_proj".into(),
    ]
}
fn default_learning_rate() -> f64 {
    2e-4
}
fn default_batch_size() -> i32 {
    2
}
fn default_grad_accum() -> i32 {
    4
}
fn default_epochs() -> i32 {
    3
}
fn default_warmup() -> i32 {
    10
}
fn default_optim() -> String {
    "adamw_8bit".into()
}
fn default_scheduler() -> String {
    "cosine".into()
}
fn default_seq_length() -> i32 {
    2048
}

/// Training metrics (stored in training_jobs.metrics).
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS, ToSchema)]
#[ts(export)]
pub struct TrainingMetrics {
    #[serde(default)]
    pub train_loss: Option<f64>,
    #[serde(default)]
    pub train_steps: Option<i64>,
    #[serde(default)]
    pub train_runtime: Option<f64>,
    #[serde(default)]
    pub train_samples_per_second: Option<f64>,
    #[serde(default)]
    pub estimated_cost: Option<f64>,
    /// Phase-specific sub-metrics (e.g., "dpo", "grpo").
    #[serde(flatten)]
    #[ts(flatten)]
    #[schema(value_type = Object)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Evaluation scores (stored in models.eval_scores and evaluations.scores).
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS, ToSchema)]
#[ts(export)]
pub struct EvaluationScores {
    #[serde(default)]
    pub domain: Option<DomainScores>,
    #[serde(default)]
    pub general: Option<GeneralScores>,
    #[serde(default)]
    pub ab_comparison: Option<ABComparisonScores>,
    #[serde(default)]
    pub safety: Option<SafetyScores>,
    #[serde(default)]
    pub overall: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DomainScores {
    pub accuracy: f64,
    pub completeness: f64,
    pub faithfulness: f64,
    pub mean: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct GeneralScores {
    pub finetuned_score: f64,
    pub base_score: f64,
    pub delta_pct: f64,
    pub forgetting_alert: bool,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub per_category: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ABComparisonScores {
    pub win_rate: f64,
    pub confidence_low: f64,
    pub confidence_high: f64,
    #[serde(default)]
    pub wins: Option<i64>,
    #[serde(default)]
    pub total: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct SafetyScores {
    pub refusal_rate: f64,
    pub base_refusal_rate: f64,
    pub degraded: bool,
    #[serde(default)]
    pub ft_refused: Option<i64>,
    #[serde(default)]
    pub base_refused: Option<i64>,
    #[serde(default)]
    pub total: Option<i64>,
}

/// Per-tenant LLM provider configuration (stored in tenants.settings.llm).
///
/// Used by Python workers to resolve LLM credentials at activity execution time.
/// API keys are stored in the DB JSONB but never returned via the API — see
/// `LlmSettingsResponse` in the dto layer for the masked version.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS, ToSchema)]
#[ts(export)]
pub struct LlmProviderConfig {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<i32>,
}

/// Top-level tenant settings structure (stored in tenants.settings JSONB).
///
/// Extensible — new config namespaces are added as optional fields.
/// No migration needed: the `settings` column already exists.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS, ToSchema)]
#[ts(export)]
pub struct TenantSettings {
    #[serde(default)]
    pub llm: Option<LlmProviderConfig>,
}

/// Deployment configuration (stored in models.deployment_config).
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS, ToSchema)]
#[ts(export)]
pub struct DeploymentConfig {
    /// Adapter reference used in inference requests (the "model" field).
    #[serde(default)]
    pub adapter_ref: Option<String>,
    /// Base model this adapter was fine-tuned from.
    #[serde(default)]
    pub base_model: Option<String>,
    /// Serving engine type (vllm, tgi, sglang).
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub deployed_at: Option<String>,
    /// Extra config not in the schema.
    #[serde(flatten)]
    #[ts(flatten)]
    #[schema(value_type = Object)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyperparams_defaults() {
        let hp: Hyperparams = serde_json::from_str("{}").unwrap();
        assert_eq!(hp.r, 16);
        assert_eq!(hp.lora_alpha, 16);
        assert_eq!(hp.learning_rate, 2e-4);
        assert_eq!(hp.target_modules.len(), 7);
    }

    #[test]
    fn hyperparams_with_overrides() {
        let hp: Hyperparams = serde_json::from_str(r#"{"r": 32, "lora_alpha": 64}"#).unwrap();
        assert_eq!(hp.r, 32);
        assert_eq!(hp.lora_alpha, 64);
        // Defaults still apply for unset fields
        assert_eq!(hp.learning_rate, 2e-4);
    }

    #[test]
    fn eval_scores_roundtrip() {
        let scores = EvaluationScores {
            domain: Some(DomainScores {
                accuracy: 4.2,
                completeness: 3.8,
                faithfulness: 4.5,
                mean: 4.17,
            }),
            general: None,
            ab_comparison: None,
            safety: None,
            overall: Some(78.5),
        };
        let json = serde_json::to_string(&scores).unwrap();
        let parsed: EvaluationScores = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.overall, Some(78.5));
    }

    #[test]
    fn deployment_config_roundtrip() {
        let config = DeploymentConfig {
            adapter_ref: Some("adapter-abc123".into()),
            base_model: Some("meta-llama/Llama-3.1-8B".into()),
            backend: Some("vllm".into()),
            deployed_at: Some("2026-01-01T00:00:00Z".into()),
            extra: HashMap::new(),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: DeploymentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.adapter_ref, Some("adapter-abc123".into()));
        assert_eq!(parsed.backend, Some("vllm".into()));
    }
}
