use uuid::Uuid;

use crate::dto::tenant_settings::{LlmSettingsResponse, UpdateLlmSettingsRequest, mask_api_key};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::TenantRepository;

/// Business logic for per-tenant configuration.
///
/// Settings are stored in the `tenants.settings` JSONB column, namespaced
/// by feature (e.g., `settings.llm` for LLM provider config). This avoids
/// new tables and migrations — the column already exists.
pub struct TenantSettingsService;

impl TenantSettingsService {
    /// Get the LLM provider settings for a tenant (API key masked).
    pub async fn get_llm_settings(
        tenant_repo: &dyn TenantRepository,
        tenant_id: Uuid,
    ) -> AppResult<LlmSettingsResponse> {
        let settings = tenant_repo.get_settings(tenant_id).await?;
        let llm = settings.get("llm");

        match llm {
            Some(llm_val) if !llm_val.is_null() => Ok(LlmSettingsResponse {
                provider: json_str(llm_val, "provider"),
                api_base_url: json_str(llm_val, "api_base_url"),
                api_key_masked: llm_val
                    .get("api_key")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(mask_api_key),
                model: json_str(llm_val, "model"),
                max_tokens: llm_val
                    .get("max_tokens")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32),
                is_configured: true,
            }),
            _ => Ok(LlmSettingsResponse {
                provider: None,
                api_base_url: None,
                api_key_masked: None,
                model: None,
                max_tokens: None,
                is_configured: false,
            }),
        }
    }

    /// Update the LLM provider settings for a tenant.
    ///
    /// Merge semantics: only provided fields are updated. Existing fields
    /// not in the request are preserved.
    pub async fn update_llm_settings(
        tenant_repo: &dyn TenantRepository,
        tenant_id: Uuid,
        request: UpdateLlmSettingsRequest,
    ) -> AppResult<LlmSettingsResponse> {
        // Validate URL if provided
        if let Some(ref url) = request.api_base_url
            && !url.starts_with("http://")
            && !url.starts_with("https://")
        {
            return Err(AppError::BadRequest {
                message: "api_base_url must start with http:// or https://".into(),
            });
        }

        // Fetch existing settings and merge
        let existing = tenant_repo.get_settings(tenant_id).await?;
        let existing_llm = existing
            .get("llm")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let mut llm_obj = existing_llm.as_object().cloned().unwrap_or_default();

        if let Some(provider) = request.provider {
            llm_obj.insert("provider".into(), serde_json::Value::String(provider));
        }
        if let Some(api_base_url) = request.api_base_url {
            llm_obj.insert(
                "api_base_url".into(),
                serde_json::Value::String(api_base_url),
            );
        }
        if let Some(api_key) = request.api_key {
            llm_obj.insert("api_key".into(), serde_json::Value::String(api_key));
        }
        if let Some(model) = request.model {
            llm_obj.insert("model".into(), serde_json::Value::String(model));
        }
        if let Some(max_tokens) = request.max_tokens {
            llm_obj.insert("max_tokens".into(), serde_json::json!(max_tokens));
        }

        let settings_update = serde_json::json!({ "llm": llm_obj });
        tenant_repo
            .update_settings(tenant_id, settings_update)
            .await?;

        Self::get_llm_settings(tenant_repo, tenant_id).await
    }

    /// Delete (clear) the LLM provider settings for a tenant.
    ///
    /// Resets to platform defaults (worker env vars).
    pub async fn delete_llm_settings(
        tenant_repo: &dyn TenantRepository,
        tenant_id: Uuid,
    ) -> AppResult<()> {
        let settings_update = serde_json::json!({ "llm": null });
        tenant_repo
            .update_settings(tenant_id, settings_update)
            .await
    }
}

/// Extract an optional string from a JSON value by key.
fn json_str(val: &serde_json::Value, key: &str) -> Option<String> {
    val.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_str_extracts_value() {
        let val = serde_json::json!({"provider": "openai", "model": ""});
        assert_eq!(json_str(&val, "provider"), Some("openai".into()));
        assert_eq!(json_str(&val, "model"), None); // empty string → None
        assert_eq!(json_str(&val, "missing"), None);
    }
}
