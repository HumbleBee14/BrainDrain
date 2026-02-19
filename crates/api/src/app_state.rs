use platform_storage::s3::{S3Config, S3Storage};
use sqlx::PgPool;
use std::sync::Arc;

use crate::config::Config;

/// Shared application state available to all route handlers.
///
/// Wrapped in `Arc` for cheap cloning across async tasks.
/// Access in handlers via `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    pub config: Config,
    pub db: PgPool,
    pub redis: redis::aio::ConnectionManager,
    pub storage: S3Storage,
}

impl AppState {
    /// Build application state from config.
    ///
    /// Initializes all connections: database pool, Redis, S3 client.
    /// Fails fast if any connection cannot be established.
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        // Database
        let db = platform_db::create_pool(&config.database_url, config.database_max_connections)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to PostgreSQL: {e}"))?;

        tracing::info!("Connected to PostgreSQL");

        // Redis
        let redis_client = redis::Client::open(config.redis_url.as_str())
            .map_err(|e| anyhow::anyhow!("Invalid Redis URL: {e}"))?;
        let redis = redis::aio::ConnectionManager::new(redis_client)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to Redis: {e}"))?;

        tracing::info!("Connected to Redis");

        // S3 / Object storage
        let storage = S3Storage::new(S3Config {
            endpoint: config.s3_endpoint.clone(),
            access_key: config.s3_access_key.clone(),
            secret_key: config.s3_secret_key.clone(),
            region: config.s3_region.clone(),
            bucket: config.s3_bucket.clone(),
            force_path_style: config.s3_endpoint.is_some(), // Force path-style for MinIO/R2
        })
        .await;

        tracing::info!("S3 client initialized (bucket: {})", &config.s3_bucket);

        Ok(Self {
            inner: Arc::new(AppStateInner {
                config,
                db,
                redis,
                storage,
            }),
        })
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub fn db(&self) -> &PgPool {
        &self.inner.db
    }

    pub fn redis(&self) -> redis::aio::ConnectionManager {
        self.inner.redis.clone()
    }

    pub fn storage(&self) -> &S3Storage {
        &self.inner.storage
    }
}
