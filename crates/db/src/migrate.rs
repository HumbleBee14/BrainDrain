use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let database_url =
        env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    println!("Running migrations against: {}", &database_url[..database_url.find('@').unwrap_or(database_url.len())]);

    let pool = platform_db::create_pool(&database_url, 5).await?;
    platform_db::run_migrations(&pool).await?;

    println!("Migrations completed successfully.");
    Ok(())
}
