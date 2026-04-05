use serde::Deserialize;

/// Application configuration, deserialized from environment variables.
///
/// Uses `envy` to map env vars to struct fields. Naming convention:
/// Struct field `database_url` maps to env var `DATABASE_URL`.
/// All required fields will cause a startup failure if missing.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Application name for logging and tracing.
    #[serde(default = "default_app_name")]
    pub app_name: String,

    /// Environment: development, staging, production.
    #[serde(default = "default_environment")]
    pub environment: String,

    /// Log level: trace, debug, info, warn, error.
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// API server bind host.
    #[serde(default = "default_host")]
    #[allow(dead_code)]
    pub api_host: String,

    /// API server bind port.
    #[serde(default = "default_port")]
    pub api_port: u16,

    // ── Database ──
    /// PostgreSQL connection URL.
    pub database_url: String,

    /// Maximum number of database connections in the pool.
    #[serde(default = "default_max_connections")]
    pub database_max_connections: u32,

    // ── Redis ──
    /// Redis connection URL.
    #[serde(default = "default_redis_url")]
    pub redis_url: String,

    // ── S3 / Object Storage ──
    /// S3 endpoint URL. `None` for real AWS S3, URL for MinIO/R2.
    pub s3_endpoint: Option<String>,

    /// S3 access key ID.
    pub s3_access_key: String,

    /// S3 secret access key.
    pub s3_secret_key: String,

    /// S3 bucket name.
    #[serde(default = "default_bucket")]
    pub s3_bucket: String,

    /// S3 region.
    #[serde(default = "default_region")]
    pub s3_region: String,

    // ── Auth (Clerk) ──
    /// Clerk JWKS URL for JWT verification.
    #[serde(default)]
    pub clerk_jwks_url: String,

    /// Clerk secret key (for server-side API calls).
    #[serde(default)]
    #[allow(dead_code)]
    pub clerk_secret_key: String,

    // ── Temporal ──
    /// Temporal server host:port.
    #[serde(default = "default_temporal_host")]
    pub temporal_host: String,

    /// Temporal namespace.
    #[serde(default = "default_temporal_namespace")]
    pub temporal_namespace: String,

    // ── Inference Limits ──
    /// Hard cap on max_tokens per inference request to prevent GPU abuse.
    /// Default: 8192. Override with INFERENCE_MAX_TOKENS env var.
    #[serde(default = "default_inference_max_tokens")]
    pub inference_max_tokens: i64,

    // ── Inference Backend ──
    /// Serving engine type. Supported: `vllm` (default), `tgi`, `sglang`.
    /// Set via INFERENCE_BACKEND_TYPE env var.
    #[serde(default = "default_inference_backend_type")]
    pub inference_backend_type: String,

    /// Base URL for the inference serving engine (INFERENCE_SERVER_URL).
    #[serde(default = "default_inference_server_url")]
    pub inference_server_url: String,

    /// Maximum number of LoRA adapters served simultaneously.
    /// Must match `--max-loras` on vLLM or equivalent for other engines.
    #[serde(default = "default_vllm_max_loras")]
    pub vllm_max_loras: i64,

    // ── Operational Knobs ──
    /// Billing batcher: channel capacity (events buffered in memory).
    #[serde(default = "default_billing_channel_capacity")]
    pub billing_channel_capacity: usize,

    /// Billing batcher: flush after this many events accumulate.
    #[serde(default = "default_billing_batch_size")]
    pub billing_batch_size: usize,

    /// Billing batcher: flush interval in seconds (even if batch not full).
    #[serde(default = "default_billing_flush_interval_secs")]
    pub billing_flush_interval_secs: u64,

    /// Notification delivery worker: poll interval in seconds.
    #[serde(default = "default_delivery_poll_interval_secs")]
    pub delivery_poll_interval_secs: u64,

    /// Stale deploy reap threshold in minutes.
    #[serde(default = "default_deploy_stale_minutes")]
    pub deploy_stale_minutes: i64,

    // ── Circuit Breaker (Inference Backend) ──
    /// Number of consecutive failures before the circuit breaker trips.
    #[serde(default = "default_cb_failure_threshold")]
    pub vllm_cb_failure_threshold: u32,

    /// Seconds to wait before probing the inference backend after the circuit breaker trips.
    #[serde(default = "default_cb_recovery_timeout_secs")]
    pub vllm_cb_recovery_timeout_secs: u64,

    // ── CORS ──
    /// Comma-separated list of allowed CORS origins.
    #[serde(default = "default_cors_origins")]
    pub cors_origins: String,

    // ── IP Rate Limiting ──
    /// Whether global IP-based rate limiting is enabled.
    #[serde(default = "default_rate_limit_enabled")]
    pub rate_limit_enabled: bool,

    /// Maximum requests per minute per IP address.
    #[serde(default = "default_rate_limit_rpm")]
    pub rate_limit_rpm: u32,

    // ── Security Headers ──
    /// Content-Security-Policy header value.
    #[serde(default = "default_csp_policy")]
    pub security_csp_policy: String,

    /// HSTS max-age in seconds (1 year default).
    #[serde(default = "default_hsts_max_age")]
    pub security_hsts_max_age: u64,

    // ── Stripe Billing ──
    /// Stripe secret key. When `None`, billing provider falls back to no-op.
    #[serde(default)]
    pub stripe_secret_key: Option<String>,

    /// Stripe webhook signing secret for signature verification.
    #[serde(default)]
    pub stripe_webhook_secret: Option<String>,

    /// Stripe price ID for the starter plan.
    #[serde(default)]
    pub stripe_price_starter: Option<String>,

    /// Stripe price ID for the growth plan.
    #[serde(default)]
    pub stripe_price_growth: Option<String>,

    /// Stripe price ID for the pro plan.
    #[serde(default)]
    pub stripe_price_pro: Option<String>,

    // ── Internal Service Auth ──
    /// Shared secret for worker → API callbacks (e.g., deploy after training).
    /// Must match APP_PLATFORM_INTERNAL_TOKEN on the worker side.
    /// When empty, internal auth is disabled.
    #[serde(default)]
    pub platform_internal_token: String,

    // ── Temporal Task Queue ──
    /// Default Temporal task queue for non-GPU workflows.
    /// Must match APP_TEMPORAL_TASK_QUEUE on the worker side.
    #[serde(default = "default_temporal_task_queue")]
    pub temporal_task_queue: String,

    // ── Observability (OTEL) ──
    /// Whether OpenTelemetry export is enabled.
    #[serde(default)]
    pub otel_enabled: bool,

    /// OTEL Collector gRPC endpoint.
    #[serde(default = "default_otel_endpoint")]
    pub otel_endpoint: String,

    // -- Feature Flags --
    /// Feature flag provider backend. Current supported value: `static`.
    #[serde(default = "default_feature_flags_provider")]
    pub feature_flags_provider: String,

    /// Inline JSON object of boolean flags for the static provider.
    /// Example: {"billing.outbox.enabled":true,"idempotency.enforced":false}
    #[serde(default)]
    pub feature_flags_json: Option<String>,

    /// Optional path to a JSON file of boolean flags for the static provider.
    #[serde(default)]
    pub feature_flags_file: Option<String>,

    /// Unleash server URL (required when FEATURE_FLAGS_PROVIDER=unleash).
    #[serde(default)]
    pub unleash_url: Option<String>,

    /// Unleash API token (required when FEATURE_FLAGS_PROVIDER=unleash).
    #[serde(default)]
    pub unleash_api_token: Option<String>,

    /// Unleash application name (sent in API requests for server-side filtering).
    #[serde(default = "default_unleash_app_name")]
    pub unleash_app_name: String,

    /// Unleash environment name (e.g., "development", "production").
    #[serde(default = "default_unleash_environment")]
    pub unleash_environment: String,
}

impl Config {
    /// Load configuration from environment variables.
    /// Loads `.env` file if present (dev mode).
    pub fn from_env() -> Result<Self, envy::Error> {
        dotenvy::dotenv().ok();
        envy::from_env::<Self>()
    }

    /// Parse CORS origins into a Vec.
    pub fn cors_origins_list(&self) -> Vec<String> {
        self.cors_origins
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Whether we're running in development mode.
    pub fn is_dev(&self) -> bool {
        self.environment == "development"
    }

    /// Build a Config with all defaults populated, only requiring the fields
    /// that have no default (database_url, s3 keys). Used by tests so they
    /// don't break every time a new field is added to Config.
    #[cfg(test)]
    pub fn test_default() -> Self {
        // Deserialize from a minimal set of key-value pairs.
        // All other fields use their serde defaults.
        let pairs: Vec<(String, String)> = vec![
            (
                "DATABASE_URL".into(),
                "postgres://test:test@localhost/test".into(),
            ),
            ("S3_ACCESS_KEY".into(), "test-key".into()),
            ("S3_SECRET_KEY".into(), "test-secret".into()),
        ];
        envy::from_iter(pairs).expect("Config::test_default should always work")
    }
}

fn default_app_name() -> String {
    "Platform API".to_string()
}
fn default_environment() -> String {
    "production".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_host() -> String {
    "0.0.0.0".to_string()
}
fn default_port() -> u16 {
    8000
}
fn default_max_connections() -> u32 {
    20
}
fn default_redis_url() -> String {
    "redis://localhost:6379".to_string()
}
fn default_bucket() -> String {
    "platform-dev".to_string()
}
fn default_region() -> String {
    "us-east-1".to_string()
}
fn default_temporal_host() -> String {
    "localhost:7233".to_string()
}
fn default_temporal_namespace() -> String {
    "default".to_string()
}
fn default_inference_backend_type() -> String {
    "vllm".to_string()
}
fn default_inference_server_url() -> String {
    "http://localhost:8080".to_string()
}
fn default_cors_origins() -> String {
    "http://localhost:3000".to_string()
}
fn default_csp_policy() -> String {
    "default-src 'self'".to_string()
}
fn default_hsts_max_age() -> u64 {
    31536000
}
fn default_otel_endpoint() -> String {
    "http://localhost:4317".to_string()
}
fn default_feature_flags_provider() -> String {
    "static".to_string()
}
fn default_unleash_app_name() -> String {
    "platform-api".to_string()
}
fn default_unleash_environment() -> String {
    "development".to_string()
}
fn default_rate_limit_enabled() -> bool {
    true
}
fn default_rate_limit_rpm() -> u32 {
    200
}
fn default_cb_failure_threshold() -> u32 {
    5
}
fn default_cb_recovery_timeout_secs() -> u64 {
    30
}
fn default_inference_max_tokens() -> i64 {
    8192
}
fn default_vllm_max_loras() -> i64 {
    4
}
fn default_billing_channel_capacity() -> usize {
    10_000
}
fn default_billing_batch_size() -> usize {
    1_000
}
fn default_billing_flush_interval_secs() -> u64 {
    5
}
fn default_delivery_poll_interval_secs() -> u64 {
    10
}
fn default_deploy_stale_minutes() -> i64 {
    10
}
fn default_temporal_task_queue() -> String {
    "ml-pipeline".to_string()
}
