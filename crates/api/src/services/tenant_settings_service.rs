use uuid::Uuid;

use crate::dto::tenant_settings::{
    AdminConfigResponse, LlmSettingsResponse, UpdateAdminConfigRequest, UpdateLlmSettingsRequest,
    mask_api_key,
};
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

    /// Get the admin configuration for a tenant, with platform defaults for unset values.
    pub async fn get_admin_config(
        tenant_repo: &dyn TenantRepository,
        tenant_id: Uuid,
    ) -> AppResult<AdminConfigResponse> {
        let settings = tenant_repo.get_settings(tenant_id).await?;
        let admin = settings.get("admin");

        let default_gpu_rates: std::collections::HashMap<String, f64> =
            platform_shared::constants::GPU_HOURLY_RATES
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect();

        match admin {
            Some(val) if !val.is_null() => {
                let gpu_rates = val
                    .get("gpu_rates")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(default_gpu_rates);

                Ok(AdminConfigResponse {
                    gpu_rates,
                    cost_approval_threshold: json_f64(val, "cost_approval_threshold")
                        .unwrap_or(5.0),
                    inference_input_cost_per_million: json_f64(
                        val,
                        "inference_input_cost_per_million",
                    )
                    .unwrap_or(0.15),
                    inference_output_cost_per_million: json_f64(
                        val,
                        "inference_output_cost_per_million",
                    )
                    .unwrap_or(0.60),
                    default_max_tokens: json_i32(val, "default_max_tokens").unwrap_or(512),
                    default_rate_limit_rpm: json_i32(val, "default_rate_limit_rpm").unwrap_or(60),
                    max_batch_size: json_i32(val, "max_batch_size").unwrap_or(50),
                    chunk_size_tokens: json_i32(val, "chunk_size_tokens").unwrap_or(1500),
                    is_configured: true,
                })
            }
            _ => Ok(AdminConfigResponse {
                gpu_rates: default_gpu_rates,
                cost_approval_threshold: 5.0,
                inference_input_cost_per_million: 0.15,
                inference_output_cost_per_million: 0.60,
                default_max_tokens: 512,
                default_rate_limit_rpm: 60,
                max_batch_size: 50,
                chunk_size_tokens: 1500,
                is_configured: false,
            }),
        }
    }

    /// Update the admin configuration for a tenant (merge semantics).
    pub async fn update_admin_config(
        tenant_repo: &dyn TenantRepository,
        tenant_id: Uuid,
        request: UpdateAdminConfigRequest,
    ) -> AppResult<AdminConfigResponse> {
        Self::validate_admin_config(&request)?;

        let existing = tenant_repo.get_settings(tenant_id).await?;
        let existing_admin = existing
            .get("admin")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let mut admin_obj = existing_admin.as_object().cloned().unwrap_or_default();

        if let Some(gpu_rates) = request.gpu_rates {
            admin_obj.insert(
                "gpu_rates".into(),
                serde_json::to_value(gpu_rates).unwrap_or_else(|_| serde_json::json!({})),
            );
        }
        if let Some(v) = request.cost_approval_threshold {
            admin_obj.insert("cost_approval_threshold".into(), serde_json::json!(v));
        }
        if let Some(v) = request.inference_input_cost_per_million {
            admin_obj.insert(
                "inference_input_cost_per_million".into(),
                serde_json::json!(v),
            );
        }
        if let Some(v) = request.inference_output_cost_per_million {
            admin_obj.insert(
                "inference_output_cost_per_million".into(),
                serde_json::json!(v),
            );
        }
        if let Some(v) = request.default_max_tokens {
            admin_obj.insert("default_max_tokens".into(), serde_json::json!(v));
        }
        if let Some(v) = request.default_rate_limit_rpm {
            admin_obj.insert("default_rate_limit_rpm".into(), serde_json::json!(v));
        }
        if let Some(v) = request.max_batch_size {
            admin_obj.insert("max_batch_size".into(), serde_json::json!(v));
        }
        if let Some(v) = request.chunk_size_tokens {
            admin_obj.insert("chunk_size_tokens".into(), serde_json::json!(v));
        }

        let settings_update = serde_json::json!({ "admin": admin_obj });
        tenant_repo
            .update_settings(tenant_id, settings_update)
            .await?;

        Self::get_admin_config(tenant_repo, tenant_id).await
    }

    /// Validate admin config values are within safe bounds.
    fn validate_admin_config(request: &UpdateAdminConfigRequest) -> AppResult<()> {
        if let Some(ref gpu_rates) = request.gpu_rates {
            for (gpu, rate) in gpu_rates {
                if *rate <= 0.0 || *rate > 100.0 {
                    return Err(AppError::BadRequest {
                        message: format!(
                            "GPU rate for '{gpu}' must be between 0 and $100/hr (got ${rate:.2})"
                        ),
                    });
                }
            }
        }
        if let Some(v) = request.cost_approval_threshold
            && !(0.0..=10_000.0).contains(&v)
        {
            return Err(AppError::BadRequest {
                message: "cost_approval_threshold must be between $0 and $10,000".into(),
            });
        }
        if let Some(v) = request.inference_input_cost_per_million
            && (v <= 0.0 || v > 100.0)
        {
            return Err(AppError::BadRequest {
                message: "inference_input_cost_per_million must be between 0 and $100".into(),
            });
        }
        if let Some(v) = request.inference_output_cost_per_million
            && (v <= 0.0 || v > 100.0)
        {
            return Err(AppError::BadRequest {
                message: "inference_output_cost_per_million must be between 0 and $100".into(),
            });
        }
        if let Some(v) = request.default_max_tokens
            && !(1..=32_768).contains(&v)
        {
            return Err(AppError::BadRequest {
                message: "default_max_tokens must be between 1 and 32,768".into(),
            });
        }
        if let Some(v) = request.default_rate_limit_rpm
            && !(1..=10_000).contains(&v)
        {
            return Err(AppError::BadRequest {
                message: "default_rate_limit_rpm must be between 1 and 10,000".into(),
            });
        }
        if let Some(v) = request.max_batch_size
            && !(1..=500).contains(&v)
        {
            return Err(AppError::BadRequest {
                message: "max_batch_size must be between 1 and 500".into(),
            });
        }
        if let Some(v) = request.chunk_size_tokens
            && !(100..=32_000).contains(&v)
        {
            return Err(AppError::BadRequest {
                message: "chunk_size_tokens must be between 100 and 32,000".into(),
            });
        }
        Ok(())
    }

    /// Reset admin configuration to platform defaults.
    pub async fn reset_admin_config(
        tenant_repo: &dyn TenantRepository,
        tenant_id: Uuid,
    ) -> AppResult<()> {
        let settings_update = serde_json::json!({ "admin": null });
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

fn json_f64(val: &serde_json::Value, key: &str) -> Option<f64> {
    val.get(key).and_then(|v| v.as_f64())
}

fn json_i32(val: &serde_json::Value, key: &str) -> Option<i32> {
    val.get(key).and_then(|v| v.as_i64()).map(|v| v as i32)
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
