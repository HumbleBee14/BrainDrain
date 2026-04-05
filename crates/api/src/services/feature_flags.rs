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

// ─── Static Provider ─────────────────────────────────────────────────────

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
    fn bool_variation(&self, key: &str, default: bool, _context: &FlagContext) -> bool {
        self.flags.get(key).copied().unwrap_or(default)
    }
}

// ─── Unleash Provider ────────────────────────────────────────────────────

/// Remote feature flag provider that polls an Unleash-compatible server.
///
/// Implements the Unleash Client Features API:
/// `GET /api/client/features` with `Authorization: <api-token>` header.
///
/// Behavior:
/// - On startup: attempts one synchronous fetch. If Unleash is unreachable,
///   falls back to static defaults from config (does not crash).
/// - Background poller: refreshes flags every `poll_interval` seconds.
/// - On poll failure: keeps serving from the last successfully fetched cache.
/// - On flag change: logs the diff (old → new) for audit trail.
/// - On shutdown: poller stops cleanly via shutdown signal.
///
/// No external SDK dependency — just reqwest + serde. The Unleash client
/// features API returns a simple JSON payload we parse ourselves.
pub struct UnleashProvider {
    /// Current flag values. Updated atomically by the background poller.
    cache: Arc<std::sync::RwLock<HashMap<String, bool>>>,
    /// Fallback values used when Unleash has never successfully responded.
    fallback: HashMap<String, bool>,
}

impl UnleashProvider {
    /// Create a new provider and attempt an initial fetch.
    ///
    /// If the initial fetch fails, the provider starts with fallback values
    /// from the static config. The background poller will keep retrying.
    pub async fn new(config: &Config) -> anyhow::Result<(Self, UnleashPollerHandle)> {
        let url = config
            .unleash_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("UNLEASH_URL is required for unleash provider"))?;
        let token = config
            .unleash_api_token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("UNLEASH_API_TOKEN is required for unleash provider"))?;
        let app_name = &config.unleash_app_name;
        let environment = &config.unleash_environment;

        // Build the static fallback from config (same source as StaticProvider).
        // Fail fast if the fallback config is invalid — a broken fallback defeats
        // the purpose of graceful degradation when Unleash is unreachable.
        let fallback = StaticFeatureFlagProvider::from_config(config)?.flags;

        let features_url = format!("{}/api/client/features", url.trim_end_matches('/'));
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                if let Ok(v) = app_name.parse() {
                    h.insert("Unleash-AppName", v);
                }
                if let Ok(v) = environment.parse() {
                    h.insert("Unleash-InstanceId", v);
                }
                h
            })
            .build()?;

        // Attempt initial fetch — non-fatal on failure
        let initial_flags = match fetch_unleash_flags(&http, &features_url, token).await {
            Ok(flags) => {
                tracing::info!(
                    flag_count = flags.len(),
                    "Unleash: initial fetch successful"
                );
                flags
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    fallback_count = fallback.len(),
                    "Unleash: initial fetch failed, using static fallback"
                );
                fallback.clone()
            }
        };

        let cache = Arc::new(std::sync::RwLock::new(initial_flags));

        let provider = Self {
            cache: Arc::clone(&cache),
            fallback: fallback.clone(),
        };

        // Start background poller
        let poll_interval = std::time::Duration::from_secs(15);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let poller_cache = Arc::clone(&cache);
        let poller_url = features_url.clone();
        let poller_token = token.to_string();
        let poller_http = http.clone();

        tokio::spawn(async move {
            unleash_poll_loop(
                poller_http,
                poller_url,
                poller_token,
                poller_cache,
                poll_interval,
                shutdown_rx,
            )
            .await;
        });

        let handle = UnleashPollerHandle {
            shutdown_tx: Some(shutdown_tx),
        };

        Ok((provider, handle))
    }
}

impl FeatureFlagProvider for UnleashProvider {
    fn bool_variation(&self, key: &str, default: bool, _context: &FlagContext) -> bool {
        let cache = self.cache.read().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = cache.get(key) {
            return *value;
        }
        // Key not in cache — check fallback, then use caller's default
        self.fallback.get(key).copied().unwrap_or(default)
    }
}

/// Handle to stop the Unleash background poller on shutdown.
pub struct UnleashPollerHandle {
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl UnleashPollerHandle {
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Background poll loop for Unleash flag updates.
async fn unleash_poll_loop(
    http: reqwest::Client,
    features_url: String,
    token: String,
    cache: Arc<std::sync::RwLock<HashMap<String, bool>>>,
    poll_interval: std::time::Duration,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut interval = tokio::time::interval(poll_interval);
    // Skip the first tick (we already fetched on startup)
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                match fetch_unleash_flags(&http, &features_url, &token).await {
                    Ok(new_flags) => {
                        // Diff and log changes
                        let old_flags = cache.read()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone();
                        log_flag_changes(&old_flags, &new_flags);

                        // Atomic swap
                        let mut cache_write = cache.write()
                            .unwrap_or_else(|e| e.into_inner());
                        *cache_write = new_flags;
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "Unleash: poll failed, serving from cache"
                        );
                    }
                }
            }
            _ = &mut shutdown_rx => {
                tracing::info!("Unleash poller stopped");
                return;
            }
        }
    }
}

/// Fetch feature flags from an Unleash-compatible `/api/client/features` endpoint.
///
/// This provider only supports **global kill switches** (enabled/disabled for
/// everyone). Features that use Unleash activation strategies, constraints, or
/// variants are skipped with a warning — they require a full Unleash SDK
/// evaluator which we intentionally do not implement.
///
/// A feature is treated as a global boolean if:
/// - It has no strategies, OR
/// - It has exactly one strategy named "default" with no constraints
///
/// Any other configuration is logged and skipped to prevent silent
/// mis-evaluation (e.g., enabling a flag for everyone when it was intended
/// for 10% of tenants).
async fn fetch_unleash_flags(
    http: &reqwest::Client,
    features_url: &str,
    token: &str,
) -> anyhow::Result<HashMap<String, bool>> {
    let resp = http
        .get(features_url)
        .header("Authorization", token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Unleash request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Unleash returned {status}: {body}"));
    }

    let payload: UnleashFeaturesResponse = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Unleash response parse failed: {e}"))?;

    let mut flags = HashMap::new();
    for feature in payload.features {
        if is_global_boolean(&feature) {
            flags.insert(feature.name, feature.enabled);
        } else {
            tracing::warn!(
                flag = feature.name,
                strategies = feature.strategies.len(),
                "Unleash: skipping flag with non-default strategies \
                 (this provider only supports global kill switches)"
            );
        }
    }

    Ok(flags)
}

/// A feature is a safe global boolean if it has no strategies or only
/// the "default" strategy with no constraints.
fn is_global_boolean(feature: &UnleashFeature) -> bool {
    if feature.strategies.is_empty() {
        return true;
    }
    if feature.strategies.len() == 1 {
        let s = &feature.strategies[0];
        return s.name == "default" && s.constraints.is_empty();
    }
    false
}

/// Log which flags changed between poll cycles for audit trail.
fn log_flag_changes(old: &HashMap<String, bool>, new: &HashMap<String, bool>) {
    // Collect all keys from both maps
    let mut all_keys: Vec<&str> = old.keys().chain(new.keys()).map(|k| k.as_str()).collect();
    all_keys.sort();
    all_keys.dedup();

    for key in all_keys {
        let old_val = old.get(key).copied();
        let new_val = new.get(key).copied();
        if old_val != new_val {
            tracing::warn!(
                flag = key,
                old_value = ?old_val,
                new_value = ?new_val,
                "Feature flag changed"
            );
        }
    }
}

/// Minimal Unleash Client Features API response shape.
#[derive(serde::Deserialize)]
struct UnleashFeaturesResponse {
    features: Vec<UnleashFeature>,
}

#[derive(serde::Deserialize)]
struct UnleashFeature {
    name: String,
    enabled: bool,
    #[serde(default)]
    strategies: Vec<UnleashStrategy>,
}

#[derive(serde::Deserialize)]
struct UnleashStrategy {
    name: String,
    #[serde(default)]
    constraints: Vec<serde_json::Value>,
}

// ─── Factory ─────────────────────────────────────────────────────────────

/// Build the feature flag facade from config.
///
/// Supported providers:
/// - `static` (default): JSON-backed, from file or inline env var.
/// - `unleash`: Polls an Unleash-compatible server. Falls back to static
///   config if Unleash is unreachable on startup.
///
/// Unsupported providers fail fast at startup.
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
        "unleash" => {
            // Unleash requires async initialization. Since build_feature_flags
            // is called during AppState::new() which is already async, we use
            // a blocking approach here: spawn a task and block on it.
            // This is safe because it only runs once at startup.
            return Err(anyhow::anyhow!(
                "Use build_feature_flags_async() for the unleash provider"
            ));
        }
        other => {
            return Err(anyhow::anyhow!(
                "Unsupported FEATURE_FLAGS_PROVIDER '{other}'. Supported: static, unleash"
            ));
        }
    };

    Ok(FeatureFlags::new(provider))
}

/// Async version of `build_feature_flags` — required for the Unleash provider
/// which performs an HTTP fetch on initialization.
pub async fn build_feature_flags_async(
    config: &Config,
) -> anyhow::Result<(FeatureFlags, Option<UnleashPollerHandle>)> {
    match config.feature_flags_provider.as_str() {
        "" | "static" => {
            let flags = build_feature_flags(config)?;
            Ok((flags, None))
        }
        "unleash" => {
            let (provider, handle) = UnleashProvider::new(config).await?;
            let mut loaded_flags: Vec<_> = provider
                .cache
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            loaded_flags.sort_by(|a, b| a.0.cmp(&b.0));
            tracing::info!(
                provider = "unleash",
                flags = ?loaded_flags,
                "Loaded feature flags"
            );
            Ok((FeatureFlags::new(Arc::new(provider)), Some(handle)))
        }
        other => Err(anyhow::anyhow!(
            "Unsupported FEATURE_FLAGS_PROVIDER '{other}'. Supported: static, unleash"
        )),
    }
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
        config.feature_flags_provider = "launchdarkly".to_string();
        let err = build_feature_flags(&config)
            .err()
            .expect("unsupported provider should fail");
        assert!(
            err.to_string()
                .contains("Unsupported FEATURE_FLAGS_PROVIDER")
        );
    }

    #[test]
    fn unleash_provider_requires_async() {
        let mut config = test_config();
        config.feature_flags_provider = "unleash".to_string();
        let err = build_feature_flags(&config)
            .err()
            .expect("unleash should require async");
        assert!(err.to_string().contains("build_feature_flags_async"));
    }

    #[test]
    fn flag_context_helpers_set_expected_fields() {
        let tenant_id = Uuid::new_v4();
        let context = FlagContext::tenant_user(tenant_id, "user-123").with_org_id("org-456");
        assert_eq!(context.tenant_id, Some(tenant_id));
        assert_eq!(context.user_id.as_deref(), Some("user-123"));
        assert_eq!(context.org_id.as_deref(), Some("org-456"));
    }

    #[test]
    fn unleash_response_parsing() {
        let json = r#"{
            "version": 2,
            "features": [
                { "name": "billing.outbox.enabled", "enabled": true },
                { "name": "idempotency.enforced", "enabled": false, "strategies": [{"name": "default"}] }
            ]
        }"#;

        let resp: UnleashFeaturesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.features.len(), 2);
        assert_eq!(resp.features[0].name, "billing.outbox.enabled");
        assert!(resp.features[0].enabled);
        assert!(resp.features[0].strategies.is_empty());
        assert_eq!(resp.features[1].name, "idempotency.enforced");
        assert!(!resp.features[1].enabled);
        assert_eq!(resp.features[1].strategies.len(), 1);
        assert_eq!(resp.features[1].strategies[0].name, "default");
    }

    #[test]
    fn is_global_boolean_accepts_no_strategies() {
        let feature = UnleashFeature {
            name: "flag.a".to_string(),
            enabled: true,
            strategies: vec![],
        };
        assert!(is_global_boolean(&feature));
    }

    #[test]
    fn is_global_boolean_accepts_default_strategy_only() {
        let feature = UnleashFeature {
            name: "flag.b".to_string(),
            enabled: true,
            strategies: vec![UnleashStrategy {
                name: "default".to_string(),
                constraints: vec![],
            }],
        };
        assert!(is_global_boolean(&feature));
    }

    #[test]
    fn is_global_boolean_rejects_custom_strategies() {
        let feature = UnleashFeature {
            name: "flag.c".to_string(),
            enabled: true,
            strategies: vec![UnleashStrategy {
                name: "gradualRollout".to_string(),
                constraints: vec![],
            }],
        };
        assert!(!is_global_boolean(&feature));
    }

    #[test]
    fn is_global_boolean_rejects_default_with_constraints() {
        let feature = UnleashFeature {
            name: "flag.d".to_string(),
            enabled: true,
            strategies: vec![UnleashStrategy {
                name: "default".to_string(),
                constraints: vec![serde_json::json!({"contextName": "tenantId"})],
            }],
        };
        assert!(!is_global_boolean(&feature));
    }

    #[test]
    fn is_global_boolean_rejects_multiple_strategies() {
        let feature = UnleashFeature {
            name: "flag.e".to_string(),
            enabled: true,
            strategies: vec![
                UnleashStrategy {
                    name: "default".to_string(),
                    constraints: vec![],
                },
                UnleashStrategy {
                    name: "userWithId".to_string(),
                    constraints: vec![],
                },
            ],
        };
        assert!(!is_global_boolean(&feature));
    }

    #[test]
    fn log_flag_changes_detects_diffs() {
        let mut old = HashMap::new();
        old.insert("flag.a".to_string(), true);
        old.insert("flag.b".to_string(), false);

        let mut new = HashMap::new();
        new.insert("flag.a".to_string(), false); // changed
        new.insert("flag.c".to_string(), true); // added

        // This just tests it doesn't panic — actual logging goes to tracing
        log_flag_changes(&old, &new);
    }
}
