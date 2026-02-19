use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::app_state::AppState;

/// Health check routes — no auth required.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct ReadyResponse {
    status: &'static str,
    database: bool,
    redis: bool,
}

/// Basic liveness check — always returns OK if the process is running.
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Readiness check — verifies database and Redis connectivity.
async fn ready(State(state): State<AppState>) -> Result<Json<ReadyResponse>, StatusCode> {
    let db_ok = sqlx::query("SELECT 1")
        .execute(state.db())
        .await
        .is_ok();

    let redis_ok = redis::cmd("PING")
        .query_async::<String>(&mut state.redis())
        .await
        .is_ok();

    let response = ReadyResponse {
        status: if db_ok && redis_ok { "ready" } else { "degraded" },
        database: db_ok,
        redis: redis_ok,
    };

    if db_ok && redis_ok {
        Ok(Json(response))
    } else {
        // Return 503 but still include the body
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}
