pub mod api_keys;
pub mod billing;
pub mod datasets;
pub mod deployments;
pub mod documents;
pub mod evaluations;
pub mod health;
pub mod inference;
pub mod pipeline;
pub mod projects;
pub mod training;
pub mod ws;

use axum::Router;

use crate::app_state::AppState;

/// Build the complete API router with all versioned routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/api/v1", v1_router())
        .merge(health::router())
        // Inference routes at /v1/ (OpenAI-compatible, API key auth)
        .merge(inference::router())
}

/// V1 API routes.
fn v1_router() -> Router<AppState> {
    Router::new()
        .merge(projects::router())
        .merge(documents::router())
        .merge(pipeline::router())
        .merge(datasets::router())
        .merge(training::router())
        .merge(evaluations::router())
        .merge(api_keys::router())
        .merge(deployments::router())
        .merge(billing::router())
        .merge(ws::router())
}
