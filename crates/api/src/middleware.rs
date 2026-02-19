use axum::http::HeaderValue;
use tower_http::cors::CorsLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

/// Build the CORS middleware from allowed origins.
pub fn cors_layer(origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();

    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
            axum::http::header::ACCEPT,
        ])
        .allow_credentials(true)
}

/// Generates and propagates X-Request-Id headers.
pub fn request_id_layers() -> (
    SetRequestIdLayer<MakeRequestUuid>,
    PropagateRequestIdLayer,
) {
    let header_name = axum::http::HeaderName::from_static("x-request-id");
    (
        SetRequestIdLayer::new(header_name.clone(), MakeRequestUuid),
        PropagateRequestIdLayer::new(header_name),
    )
}

/// HTTP request/response tracing layer.
pub fn trace_layer() -> TraceLayer<tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>> {
    TraceLayer::new_for_http()
}
