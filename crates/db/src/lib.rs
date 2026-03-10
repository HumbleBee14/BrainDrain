pub mod models;

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// Create a production-hardened connection pool to PostgreSQL.
///
/// Key settings:
/// - `min_connections`: keeps 25% of pool warm to avoid cold-start latency
/// - `acquire_timeout`: fails fast (5s) under contention instead of hanging 30s
/// - `idle_timeout`: reclaims idle connections after 5 minutes
/// - `max_lifetime`: forces connection refresh every 30 minutes (handles DB restarts)
/// - `test_before_acquire`: pings connection before use (detects stale connections)
pub async fn create_pool(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(max_connections / 4)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(300))
        .max_lifetime(Duration::from_secs(1800))
        .test_before_acquire(true)
        .connect(database_url)
        .await
}

/// Run all pending migrations against the database.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("src/migrations").run(pool).await
}
