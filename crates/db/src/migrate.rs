use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    // Redact password from URL for logging.
    // postgres://user:password@host/db → postgres://user:***@host/db
    let safe_url = if let Some(at_pos) = database_url.find('@') {
        let scheme_end = database_url.find("://").map(|p| p + 3).unwrap_or(0);
        let colon_pos = database_url[scheme_end..at_pos].find(':');
        match colon_pos {
            Some(c) => format!(
                "{}***{}",
                &database_url[..scheme_end + c + 1],
                &database_url[at_pos..]
            ),
            None => format!(
                "{}***{}",
                &database_url[..scheme_end],
                &database_url[at_pos..]
            ),
        }
    } else {
        database_url.clone()
    };
    println!("Running migrations against: {safe_url}");

    let pool = platform_db::create_pool(&database_url, 5).await?;

    // Pre-flight: show current migration state
    let applied: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(&pool)
            .await
            .unwrap_or((0,));
    println!("Applied migrations before run: {}", applied.0);

    // Run pending migrations
    platform_db::run_migrations(&pool).await?;

    // Post-flight: verify and show result
    let after: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(&pool)
            .await?;

    let new_count = after.0 - applied.0;
    if new_count > 0 {
        println!("Applied {new_count} new migration(s). Total: {}", after.0);
    } else {
        println!("No pending migrations. Total: {}", after.0);
    }

    // Ensure billing partitions exist (idempotent)
    platform_db::ensure_billing_partitions(&pool, 3).await?;
    println!("Billing partitions verified (current + 3 months ahead).");

    println!("Migration completed successfully.");
    Ok(())
}
