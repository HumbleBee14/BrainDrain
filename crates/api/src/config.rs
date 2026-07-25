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
    /// PostgreSQL connection URL for the owner/admin role. Runs migrations,
    /// partition DDL, admin endpoints, and cross-tenant maintenance.
    pub database_url: String,

    /// PostgreSQL connection URL for the least-privilege `app_rls` role that
    /// carries tenant request traffic and is subject to Row-Level Security.
    /// When unset, tenant traffic falls back to `database_url` and the RLS
    /// second layer is inactive (a startup WARN is emitted). Set this in every
    /// real deployment.
    #[serde(default)]
    pub database_rls_url: Option<String>,

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

    /// Expected JWT `iss` claim (e.g. `https://your-instance.clerk.accounts.dev`).
    /// Unset ⇒ issuer is not validated. Set this in every real deployment so
    /// tokens signed by another instance's keys are rejected.
    #[serde(default)]
    pub clerk_issuer: Option<String>,

    /// Comma-separated allowlist for the JWT `azp` (authorized party) claim —
    /// the frontend origins allowed to mint sessions. Empty ⇒ `azp` is not
    /// checked. When set, tokens without a matching `azp` are rejected.
    #[serde(default)]
    pub clerk_authorized_parties: String,

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

    /// Bearer token the serving engine requires (INFERENCE_API_KEY).
    #[serde(default)]
    pub inference_api_key: Option<String>,

    /// Request timeout for calls to the serving engine
    /// (INFERENCE_REQUEST_TIMEOUT_SECS). Far longer than the shared outbound
    /// timeout: a scale-to-zero engine must cold-start a GPU and load weights
    /// before it answers the first request.
    #[serde(default = "default_inference_request_timeout_secs")]
    pub inference_request_timeout_secs: u64,

    /// Upper bound on the total adapter bytes packaged into a download archive,
    /// which is built in memory (ADAPTER_DOWNLOAD_MAX_BYTES).
    #[serde(default = "default_adapter_download_max_bytes")]
    pub adapter_download_max_bytes: i64,

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

    /// Billing outbox: prune delivered rows older than this many days. The relay
    /// runs the prune on a coarse cadence. Set to 0 to disable pruning.
    #[serde(default = "default_billing_outbox_retention_days")]
    pub billing_outbox_retention_days: i32,

    /// Stale deploy reap threshold in minutes.
    #[serde(default = "default_deploy_stale_minutes")]
    pub deploy_stale_minutes: i64,

    /// Stale `generating` dataset reap threshold in minutes. Generous: a large
    /// document set legitimately spends a long time in LLM calls.
    #[serde(default = "default_generation_stale_minutes")]
    pub generation_stale_minutes: i64,

    /// Eval gate: block a normal deploy unless the model's latest completed
    /// evaluation shows an A/B win rate against the base model at least this
    /// high (`0.0`–`1.0`). Unset disables this rule. Rollbacks to a previously
    /// deployed version bypass the gate.
    #[serde(default)]
    pub deploy_min_ab_win_rate: Option<f64>,

    /// Eval gate: block a normal deploy when the model's general-benchmark
    /// regression against the base model exceeds this many percentage points
    /// (e.g. `10.0`). Unset disables this rule. Rollbacks bypass the gate.
    #[serde(default)]
    pub deploy_max_benchmark_regression: Option<f64>,

    /// Eval gate: block a normal deploy unless the model's document-knowledge
    /// lift over the base model (judged-mean difference on the golden holdout,
    /// 1–5 rubric) is at least this high (e.g. `0.5`). Unset disables this
    /// rule. Rollbacks bypass the gate.
    #[serde(default)]
    pub deploy_min_doc_knowledge_lift: Option<f64>,

    /// Inference instance health probe interval in seconds.
    #[serde(default = "default_inference_instance_health_poll_interval_secs")]
    pub inference_instance_health_poll_interval_secs: u64,

    /// Inference instance reconciliation interval in seconds.
    #[serde(default = "default_inference_instance_reconcile_interval_secs")]
    pub inference_instance_reconcile_interval_secs: u64,

    /// Stuck-job reaper: poll interval in seconds.
    #[serde(default = "default_reaper_poll_interval_secs")]
    pub reaper_poll_interval_secs: u64,

    /// A training/provisioning job idle this long with no live workflow is
    /// treated as abandoned (worker crash) and failed + billed for GPU used.
    #[serde(default = "default_training_stuck_timeout_secs")]
    pub training_stuck_timeout_secs: i64,

    /// A document stuck in `parsing` this long is failed with an error message.
    #[serde(default = "default_parsing_stuck_timeout_secs")]
    pub parsing_stuck_timeout_secs: i64,

    /// A model pinned in `deploying` this long is treated as an abandoned deploy
    /// (the synchronous deploy request died mid-flight): it is reset to a
    /// terminal `undeployed` state and any inference-instance slot it claimed is
    /// released, freeing capacity and unblocking redeploys. `0` disables it.
    #[serde(default = "default_deploying_stuck_timeout_secs")]
    pub deploying_stuck_timeout_secs: i64,

    /// Idle serving instances with no inference traffic for this long are scaled
    /// to zero (their models undeployed and the instance retired). `0` disables
    /// idle reaping — the default, since instances are operator-registered.
    #[serde(default = "default_inference_instance_idle_timeout_secs")]
    pub inference_instance_idle_timeout_secs: i64,

    /// A `failed` document's uploaded source object is deleted from object
    /// storage once the row has been failed this long, reclaiming storage that
    /// would otherwise leak (parsing never consumes a failed source again).
    /// `0` disables the sweep.
    #[serde(default = "default_orphaned_document_sweep_secs")]
    pub orphaned_document_sweep_secs: i64,

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

    /// Comma-separated CIDRs (or single IPs) of trusted reverse proxies.
    /// When a request's socket IP is inside one of these ranges, the client IP
    /// for rate limiting is read from X-Forwarded-For (rightmost untrusted
    /// entry). Empty ⇒ forwarded headers are ignored and the socket IP is used
    /// — behind a load balancer that means all traffic shares one bucket, so
    /// set this in every proxied deployment.
    #[serde(default)]
    pub trusted_proxy_cidrs: String,

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

    // ── Email (SMTP) ──
    /// SMTP host of an existing email provider. Unset ⇒ email disabled (sends
    /// fail visibly). Example hosts are in `.env.example`.
    #[serde(default)]
    pub smtp_host: Option<String>,

    /// SMTP port. 465 uses implicit TLS; other ports use STARTTLS.
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,

    #[serde(default)]
    pub smtp_username: Option<String>,

    #[serde(default)]
    pub smtp_password: Option<String>,

    /// From address, e.g. `Platform <noreply@yourdomain.com>`.
    #[serde(default)]
    pub email_from: Option<String>,

    // ── Internal Service Auth ──
    /// Shared secret for worker → API callbacks (e.g., deploy after training).
    /// Must match APP_PLATFORM_INTERNAL_TOKEN on the worker side.
    /// When empty, internal auth is disabled.
    #[serde(default)]
    pub platform_internal_token: String,

    // ── Secrets at Rest ──
    /// Base64-encoded 32-byte key for AES-256-GCM encryption of tenant secrets
    /// (LLM API keys) stored in the database. Generate: `openssl rand -base64 32`.
    /// Must match the workers' APP_SETTINGS_ENCRYPTION_KEY. When unset,
    /// development stores plaintext (with a warning); production refuses to
    /// store tenant API keys.
    #[serde(default)]
    pub settings_encryption_key: Option<String>,

    // ── Platform Admin ──
    /// Comma-separated allowlist of auth subject IDs (JWT `sub`) that may call
    /// platform/infrastructure admin endpoints. Empty ⇒ nobody is a platform
    /// admin (deny-all, the secure default).
    #[serde(default)]
    pub platform_admin_user_ids: String,

    /// Comma-separated allowlist of email addresses that may call platform admin
    /// endpoints. Requires the auth token to carry an `email` claim. Empty ⇒ no
    /// email grants anyone admin.
    #[serde(default)]
    pub platform_admin_emails: String,

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
    /// Feature flag provider backend. Supported: `static` (default), `unleash`.
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

    /// Unleash poll interval in seconds for remote flag refresh.
    #[serde(default = "default_unleash_poll_interval_secs")]
    pub unleash_poll_interval_secs: u64,
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
        split_csv(&self.cors_origins)
    }

    /// Serving-engine bearer token; blank is treated as unset.
    pub fn inference_api_key(&self) -> Option<&str> {
        self.inference_api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
    }

    /// Platform-admin subject IDs (JWT `sub`), parsed from the allowlist.
    pub fn platform_admin_user_ids_list(&self) -> Vec<String> {
        split_csv(&self.platform_admin_user_ids)
    }

    /// Platform-admin emails, parsed from the allowlist and lowercased for
    /// case-insensitive comparison.
    pub fn platform_admin_emails_list(&self) -> Vec<String> {
        split_csv(&self.platform_admin_emails)
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect()
    }

    /// Authorized parties for the JWT `azp` claim, parsed from the allowlist.
    pub fn clerk_authorized_parties_list(&self) -> Vec<String> {
        split_csv(&self.clerk_authorized_parties)
    }

    /// Trusted proxy CIDRs, parsed from the comma-separated list.
    pub fn trusted_proxy_cidrs_list(&self) -> Vec<String> {
        split_csv(&self.trusted_proxy_cidrs)
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

/// Split a comma-separated env value into trimmed, non-empty entries.
fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
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
fn default_unleash_poll_interval_secs() -> u64 {
    15
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
fn default_inference_request_timeout_secs() -> u64 {
    600
}
fn default_vllm_max_loras() -> i64 {
    4
}
fn default_adapter_download_max_bytes() -> i64 {
    2 * 1024 * 1024 * 1024
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
fn default_billing_outbox_retention_days() -> i32 {
    30
}
fn default_deploy_stale_minutes() -> i64 {
    10
}
fn default_generation_stale_minutes() -> i64 {
    120
}
fn default_inference_instance_health_poll_interval_secs() -> u64 {
    60
}
fn default_inference_instance_reconcile_interval_secs() -> u64 {
    60
}
fn default_reaper_poll_interval_secs() -> u64 {
    300
}
fn default_training_stuck_timeout_secs() -> i64 {
    3_600
}
fn default_parsing_stuck_timeout_secs() -> i64 {
    1_800
}
fn default_deploying_stuck_timeout_secs() -> i64 {
    600
}
fn default_inference_instance_idle_timeout_secs() -> i64 {
    0
}
fn default_orphaned_document_sweep_secs() -> i64 {
    604_800
}
fn default_temporal_task_queue() -> String {
    "ml-pipeline".to_string()
}
fn default_smtp_port() -> u16 {
    587
}
