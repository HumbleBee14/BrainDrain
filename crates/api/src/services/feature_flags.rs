use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use crate::config::Config;

/// Initial platform flags. Keep keys stable once introduced.
pub const BILLING_OUTBOX_ENABLED: &str = "billing.outbox.enabled";
pub const IDEMPOTENCY_ENFORCED: &str = "idempotency.enforced";
pub const DEPLOYMENTS_MULTI_INSTANCE_ENABLED: &str = "deployments.multi_instance.enabled";
pub const NOTIFICATIONS_DELIVERY_WORKER_ENABLED: &str = "notifications.delivery_worker.enabled";
pub const INFERENCE_BACKEND_TGI_ENABLED: &str = "inference.backend.tgi.enabled";

/// Context available during flag evaluation.
///
/// This is intentionally small for now. It is enough to support tenant- and
/// user-aware rules later without hard-coupling the application to a specific
/// provider SDK.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagContext {
    pub tenant_id: Option<Uuid>,
    pub user_id: Option<String>,
    pub org_id: Option<String>,
}

impl FlagContext {
    #[allow(dead_code)]
    pub fn tenant(tenant_id: Uuid) -> Self {
        Self {
            tenant_id: Some(tenant_id),
            ..Self::default()
        }
    }

    #[allow(dead_code)]
    pub fn tenant_user(tenant_id: Uuid, user_id: impl Into<String>) -> Self {
        Self {
            tenant_id: Some(tenant_id),
            user_id: Some(user_id.into()),
            ..Self::default()
        }
    }

    #[allow(dead_code)]
    pub fn with_org_id(mut self, org_id: impl Into<String>) -> Self {
        self.org_id = Some(org_id.into());
        self
    }
}

/// Provider abstraction for feature flag evaluation.
///
/// Application code should depend only on this trait or the `FeatureFlags`
/// facade below. That keeps vendor choice out of route and service logic.
pub trait FeatureFlagProvider: Send + Sync {
    fn bool_variation(&self, key: &str, default: bool, context: &FlagContext) -> bool;
}

/// Small facade used through `AppState`.
#[derive(Clone)]
pub struct FeatureFlags {
    provider: Arc<dyn FeatureFlagProvider>,
}

impl FeatureFlags {
    pub fn new(provider: Arc<dyn FeatureFlagProvider>) -> Self {
        Self { provider }
    }

    pub fn is_enabled(&self, key: &str, context: &FlagContext) -> bool {
        self.bool_variation(key, false, context)
    }

    pub fn bool_variation(&self, key: &str, default: bool, context: &FlagContext) -> bool {
        self.provider.bool_variation(key, default, context)
    }
}

/// Static JSON-backed provider used as the initial implementation.
///
/// Example:
/// `{ "billing.outbox.enabled": true, "idempotency.enforced": false }`
#[derive(Debug, Default)]
pub struct StaticFeatureFlagProvider {
    flags: HashMap<String, bool>,
}

impl StaticFeatureFlagProvider {
    pub fn from_config(config: &Config) -> anyhow::Result<Self> {
        if let Some(raw_path) = config.feature_flags_file.as_deref() {
            let path = raw_path.trim();
            if !path.is_empty() {
                let contents = std::fs::read_to_string(path).map_err(|e| {
                    anyhow::anyhow!("Failed to read FEATURE_FLAGS_FILE '{path}': {e}")
                })?;
                return Self::from_json(&contents);
            }
        }

        if let Some(raw_json) = config.feature_flags_json.as_deref() {
            return Self::from_json(raw_json);
        }

        Ok(Self::default())
    }

    pub fn from_json(raw_json: &str) -> anyhow::Result<Self> {
        if raw_json.trim().is_empty() {
            return Ok(Self::default());
        }

        let flags = serde_json::from_str::<HashMap<String, bool>>(raw_json)
            .map_err(|e| anyhow::anyhow!("Invalid feature flag JSON: {e}"))?;
        Ok(Self { flags })
    }
}

impl FeatureFlagProvider for StaticFeatureFlagProvider {
    /// Static flags intentionally ignore per-request context.
    ///
    /// The context parameter exists so application code can keep one evaluation
    /// API while later providers add tenant/user targeting.
    fn bool_variation(&self, key: &str, default: bool, _context: &FlagContext) -> bool {
        self.flags.get(key).copied().unwrap_or(default)
    }
}

/// Build the feature flag facade from config.
///
/// Current provider support intentionally starts with `static`.
/// Unsupported providers fail fast at startup rather than silently falling back.
pub fn build_feature_flags(config: &Config) -> anyhow::Result<FeatureFlags> {
    let provider: Arc<dyn FeatureFlagProvider> = match config.feature_flags_provider.as_str() {
        "" | "static" => {
            let static_provider = StaticFeatureFlagProvider::from_config(config)?;
            let mut loaded_flags: Vec<_> = static_provider.flags.iter().collect();
            loaded_flags.sort_by(|a, b| a.0.cmp(b.0));
            tracing::info!(
                provider = "static",
                flags = ?loaded_flags,
                "Loaded feature flags"
            );
            Arc::new(static_provider)
        }
        other => {
            return Err(anyhow::anyhow!(
                "Unsupported FEATURE_FLAGS_PROVIDER '{other}'. Supported: static"
            ));
        }
    };

    Ok(FeatureFlags::new(provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::test_default()
    }

    #[test]
    fn static_provider_defaults_to_false() {
        let provider = StaticFeatureFlagProvider::default();
        let context = FlagContext::default();
        assert!(!provider.bool_variation(BILLING_OUTBOX_ENABLED, false, &context));
        assert!(provider.bool_variation(BILLING_OUTBOX_ENABLED, true, &context));
    }

    #[test]
    fn static_provider_parses_json_flags() {
        let provider = StaticFeatureFlagProvider::from_json(
            r#"{"billing.outbox.enabled":true,"idempotency.enforced":false}"#,
        )
        .expect("valid flag json");

        let context = FlagContext::default();
        assert!(provider.bool_variation(BILLING_OUTBOX_ENABLED, false, &context));
        assert!(!provider.bool_variation(IDEMPOTENCY_ENFORCED, true, &context));
    }

    #[test]
    fn build_feature_flags_reads_inline_json() {
        let mut config = test_config();
        config.feature_flags_json =
            Some(r#"{"notifications.delivery_worker.enabled":true}"#.to_string());

        let flags = build_feature_flags(&config).expect("feature flags should build");
        assert!(flags.bool_variation(
            NOTIFICATIONS_DELIVERY_WORKER_ENABLED,
            false,
            &FlagContext::default()
        ));
    }

    #[test]
    fn unsupported_provider_fails_fast() {
        let mut config = test_config();
        config.feature_flags_provider = "unleash".to_string();
        let err = build_feature_flags(&config)
            .err()
            .expect("unsupported provider should fail");
        assert!(
            err.to_string()
                .contains("Unsupported FEATURE_FLAGS_PROVIDER")
        );
    }

    #[test]
    fn flag_context_helpers_set_expected_fields() {
        let tenant_id = Uuid::new_v4();
        let context = FlagContext::tenant_user(tenant_id, "user-123").with_org_id("org-456");
        assert_eq!(context.tenant_id, Some(tenant_id));
        assert_eq!(context.user_id.as_deref(), Some("user-123"));
        assert_eq!(context.org_id.as_deref(), Some("org-456"));
    }
}
