mod app_state;
mod auth;
mod auth_api_key;
mod config;
mod dto;
mod error;
mod middleware;
mod repositories;
mod routes;
mod services;
mod temporal;

use std::net::SocketAddr;

use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use app_state::AppState;
use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load configuration from environment
    let config = Config::from_env().map_err(|e| anyhow::anyhow!("Config error: {e}"))?;

    // Initialize structured logging
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    if config.is_dev() {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    }

    tracing::info!(
        app = %config.app_name,
        env = %config.environment,
        "Starting API server"
    );

    // Build application state (connects to DB, Redis, S3)
    let state = AppState::new(config.clone()).await?;

    // Run database migrations
    tracing::info!("Running database migrations...");
    platform_db::run_migrations(state.db()).await?;
    tracing::info!("Migrations complete");

    // Build middleware stack
    let cors_origins = config.cors_origins_list();
    let (set_request_id, propagate_request_id) = middleware::request_id_layers();

    // Build router
    let app = routes::router()
        .with_state(state)
        .layer(propagate_request_id)
        .layer(middleware::trace_layer())
        .layer(middleware::cors_layer(&cors_origins))
        .layer(set_request_id);

    // Start server
    let addr = SocketAddr::from(([0, 0, 0, 0], config.api_port));
    let listener = TcpListener::bind(addr).await?;

    tracing::info!(%addr, "Server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Server shutdown complete");
    Ok(())
}

/// Listen for Ctrl+C or SIGTERM for graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received");
}
