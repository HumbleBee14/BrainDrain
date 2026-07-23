use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Request to update tenant LLM provider settings.
///
/// All fields are optional — only provided fields are updated (merge semantics).
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateLlmSettingsRequest {
    /// Provider identifier (e.g., "openai", "anthropic", "groq", "custom")
    #[ts(optional)]
    pub provider: Option<String>,
    /// OpenAI-compatible API base URL
    #[ts(optional)]
    pub api_base_url: Option<String>,
    /// API key — stored encrypted, never returned in full
    #[ts(optional)]
    pub api_key: Option<String>,
    /// Model identifier (e.g., "gpt-4o-mini", "llama-3.1-70b-versatile")
    #[ts(optional)]
    pub model: Option<String>,
    /// Max tokens per LLM call
    #[ts(optional)]
    pub max_tokens: Option<i32>,
}

/// Response showing tenant LLM provider settings (API key masked).
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct LlmSettingsResponse {
    pub provider: Option<String>,
    pub api_base_url: Option<String>,
    /// Masked key (e.g., "sk-p...wxyz") or null if not set
    pub api_key_masked: Option<String>,
    pub model: Option<String>,
    pub max_tokens: Option<i32>,
    /// Whether the tenant has custom LLM configuration
    pub is_configured: bool,
}

/// Result of a live connectivity check against the configured LLM provider.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct LlmTestResponse {
    pub success: bool,
    pub message: String,
    /// HTTP status the provider returned, when the request completed.
    pub status_code: Option<u16>,
}

/// Request to update tenant admin configuration.
///
/// All fields are optional — only provided fields are updated (merge semantics).
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateAdminConfigRequest {
    /// GPU hourly rates as a map of gpu_class → rate_usd
    #[ts(optional)]
    pub gpu_rates: Option<std::collections::HashMap<String, f64>>,
    /// Cost threshold (USD) above which training jobs require approval
    #[ts(optional)]
    pub cost_approval_threshold: Option<f64>,
    /// Input token cost per million tokens for inference billing
    #[ts(optional)]
    pub inference_input_cost_per_million: Option<f64>,
    /// Output token cost per million tokens for inference billing
    #[ts(optional)]
    pub inference_output_cost_per_million: Option<f64>,
    /// Default max tokens per inference request
    #[ts(optional)]
    pub default_max_tokens: Option<i32>,
    /// Default rate limit (requests per minute) for new API keys
    #[ts(optional)]
    pub default_rate_limit_rpm: Option<i32>,
    /// Maximum batch inference size
    #[ts(optional)]
    pub max_batch_size: Option<i32>,
    /// Document chunk size in tokens for parsing
    #[ts(optional)]
    pub chunk_size_tokens: Option<i32>,
}

/// Response showing tenant admin configuration.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct AdminConfigResponse {
    pub gpu_rates: std::collections::HashMap<String, f64>,
    pub cost_approval_threshold: f64,
    pub inference_input_cost_per_million: f64,
    pub inference_output_cost_per_million: f64,
    pub default_max_tokens: i32,
    pub default_rate_limit_rpm: i32,
    pub max_batch_size: i32,
    pub chunk_size_tokens: i32,
    pub is_configured: bool,
}

/// Mask an API key for display: "sk-proj-abcdefg...xyz" → "sk-p...wxyz"
pub fn mask_api_key(key: &str) -> String {
    if key.len() <= 8 {
        return "****".to_string();
    }
    let prefix = &key[..4];
    let suffix = &key[key.len() - 4..];
    format!("{prefix}...{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_short_key() {
        assert_eq!(mask_api_key("abc"), "****");
        assert_eq!(mask_api_key("12345678"), "****");
    }

    #[test]
    fn mask_normal_key() {
        assert_eq!(mask_api_key("sk-proj-abcdef123456"), "sk-p...3456");
    }

    #[test]
    fn mask_nine_char_key() {
        assert_eq!(mask_api_key("123456789"), "1234...6789");
    }
}
