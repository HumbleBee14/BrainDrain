pub mod datasets;
pub mod documents;
pub mod health;
pub mod pipeline;
pub mod projects;

use axum::Router;

use crate::app_state::AppState;

/// Build the complete API router with all versioned routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/api/v1", v1_router())
        .merge(health::router())
}

/// V1 API routes.
fn v1_router() -> Router<AppState> {
    Router::new()
        .merge(projects::router())
        .merge(documents::router())
        .merge(pipeline::router())
        .merge(datasets::router())
}
