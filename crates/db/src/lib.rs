pub mod models;
pub mod tenant;

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
/// - `before_acquire`: resets `app.tenant_id` to '' on every checkout, preventing
///   stale tenant context from leaking between requests (RLS enforcement)
///
/// Actual per-request tenant context is set explicitly via `tenant::with_tenant`.
pub async fn create_pool(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(max_connections / 4)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(300))
        .max_lifetime(Duration::from_secs(1800))
        .test_before_acquire(true)
        .before_acquire(|conn, _meta| {
            Box::pin(async move {
                // Reset tenant context before handing the connection to the app.
                // This ensures a prior request's SET LOCAL doesn't bleed through.
                sqlx::query("SELECT set_config('app.tenant_id', '', false)")
                    .execute(conn)
                    .await?;
                Ok(true)
            })
        })
        .connect(database_url)
        .await
}

/// Run all pending migrations against the database.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("src/migrations").run(pool).await
}

/// Ensure billing partitions exist for the next N months.
///
/// Calls the `create_billing_partition(date)` PG function created by migration 003.
/// Idempotent — safe to call on every startup. Without this, inserts into future
/// months fail on the partitioned billing_events table.
pub async fn ensure_billing_partitions(pool: &PgPool, months_ahead: u32) -> Result<(), sqlx::Error> {
    for i in 0..months_ahead {
        sqlx::query(
            "SELECT create_billing_partition((date_trunc('month', CURRENT_DATE) + make_interval(months => $1))::date)",
        )
        .bind(i as i32)
        .execute(pool)
        .await?;
    }
    Ok(())
}
