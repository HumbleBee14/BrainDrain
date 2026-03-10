pub mod models;
pub mod tenant;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// Create a connection pool to PostgreSQL.
///
/// The pool uses a `before_acquire` hook to reset `app.tenant_id` to an empty
/// string on every connection checkout. This prevents stale tenant context from
/// leaking between requests (a pool connection is reused across many requests).
///
/// Actual per-request tenant context is set explicitly via `tenant::with_tenant`.
pub async fn create_pool(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
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
