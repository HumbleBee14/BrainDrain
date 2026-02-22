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
