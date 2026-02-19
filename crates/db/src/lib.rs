pub mod models;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Create a connection pool to PostgreSQL.
pub async fn create_pool(database_url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
}

/// Run all pending migrations against the database.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("src/migrations").run(pool).await
}
