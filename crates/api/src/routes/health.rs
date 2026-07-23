use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use utoipa::ToSchema;

use crate::app_state::AppState;
use platform_storage::ObjectStorage;

/// Key HEAD-probed to confirm object storage is reachable. It need not exist —
/// a "not found" still proves the bucket is answering; only a transport/auth
/// failure marks storage unready.
const STORAGE_PROBE_KEY: &str = "__readiness_probe__";

/// Health check routes — no auth required.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
}

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(Serialize, ToSchema)]
pub struct ReadyResponse {
    status: &'static str,
    database: bool,
    redis: bool,
    storage: bool,
}

/// Basic liveness check — always returns OK if the process is running.
#[utoipa::path(
    get,
    path = "/health",
    tag = "Health",
    responses(
        (status = 200, description = "Service is alive", body = HealthResponse),
    )
)]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Readiness check — verifies database and Redis connectivity.
#[utoipa::path(
    get,
    path = "/ready",
    tag = "Health",
    responses(
        (status = 200, description = "Service is ready", body = ReadyResponse),
        (status = 503, description = "Service degraded"),
    )
)]
pub async fn ready(State(state): State<AppState>) -> Result<Json<ReadyResponse>, StatusCode> {
    // Probe the three backing services concurrently — they are independent.
    let (db_ok, redis_ok, storage_ok) = tokio::join!(
        async { sqlx::query("SELECT 1").execute(state.db()).await.is_ok() },
        async {
            redis::cmd("PING")
                .query_async::<String>(&mut state.redis())
                .await
                .is_ok()
        },
        async { state.storage().exists(STORAGE_PROBE_KEY).await.is_ok() },
    );

    let all_ok = db_ok && redis_ok && storage_ok;
    let response = ReadyResponse {
        status: if all_ok { "ready" } else { "degraded" },
        database: db_ok,
        redis: redis_ok,
        storage: storage_ok,
    };

    if all_ok {
        Ok(Json(response))
    } else {
        // Return 503 but still include the body
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}
