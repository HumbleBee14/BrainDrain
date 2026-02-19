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
    #[allow(dead_code)]
    pub temporal_host: String,

    /// Temporal namespace.
    #[serde(default = "default_temporal_namespace")]
    #[allow(dead_code)]
    pub temporal_namespace: String,

    // ── CORS ──
    /// Comma-separated list of allowed CORS origins.
    #[serde(default = "default_cors_origins")]
    pub cors_origins: String,
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
}

fn default_app_name() -> String {
    "Platform API".to_string()
}
fn default_environment() -> String {
    "development".to_string()
}
fn default_log_level() -> String {
    "debug".to_string()
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
fn default_cors_origins() -> String {
    "http://localhost:3000".to_string()
}
