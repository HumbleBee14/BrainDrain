use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::header::{
    CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS,
    X_FRAME_OPTIONS,
};
use axum::http::{HeaderName, HeaderValue, Request, Response};
use axum::middleware::Next;
use tower_http::cors::CorsLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

/// Build the CORS middleware from allowed origins.
pub fn cors_layer(origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();

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
pub fn request_id_layers() -> (SetRequestIdLayer<MakeRequestUuid>, PropagateRequestIdLayer) {
    let header_name = axum::http::HeaderName::from_static("x-request-id");
    (
        SetRequestIdLayer::new(header_name.clone(), MakeRequestUuid),
        PropagateRequestIdLayer::new(header_name),
    )
}

/// HTTP request/response tracing layer.
pub fn trace_layer()
-> TraceLayer<tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>>
{
    TraceLayer::new_for_http()
}

// -- Security Headers --

/// Config for security headers, constructed once at startup.
#[derive(Clone)]
pub struct SecurityHeadersConfig {
    csp_policy: HeaderValue,
    hsts_value: HeaderValue,
}

impl SecurityHeadersConfig {
    pub fn new(csp_policy: &str, hsts_max_age: u64) -> Self {
        let hsts = format!("max-age={hsts_max_age}; includeSubDomains");
        Self {
            csp_policy: HeaderValue::from_str(csp_policy)
                .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'")),
            hsts_value: HeaderValue::from_str(&hsts).unwrap_or_else(|_| {
                HeaderValue::from_static("max-age=31536000; includeSubDomains")
            }),
        }
    }
}

/// Middleware that adds security headers to every response.
pub async fn security_headers(
    State(config): State<SecurityHeadersConfig>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let mut response = next.run(request).await;
    let h = response.headers_mut();

    h.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(STRICT_TRANSPORT_SECURITY, config.hsts_value.clone());
    h.insert(CONTENT_SECURITY_POLICY, config.csp_policy.clone());
    h.insert(
        REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    h.insert(
        HeaderName::from_static("x-xss-protection"),
        HeaderValue::from_static("0"),
    );

    response
}

// -- HTTP Metrics --

use opentelemetry::metrics::{Counter, Histogram};

use crate::config::Config;

/// OTEL metrics instruments for HTTP request tracking.
///
/// When `otel_enabled=true`, records actual Prometheus-compatible metrics via
/// the OTEL metrics SDK (histogram for latency, counter for requests).
/// When disabled, instruments are no-op (OTEL global meter returns no-ops).
#[derive(Clone)]
pub struct HttpMetrics {
    request_duration: Histogram<f64>,
    request_counter: Counter<u64>,
}

impl HttpMetrics {
    pub fn new(config: &Config) -> Self {
        let meter = if config.otel_enabled {
            opentelemetry::global::meter("platform-api")
        } else {
            opentelemetry::global::meter("noop")
        };

        let request_duration = meter
            .f64_histogram("http_server_request_duration_seconds")
            .with_description("HTTP request duration in seconds")
            .with_unit("s")
            .build();

        let request_counter = meter
            .u64_counter("http_server_requests_total")
            .with_description("Total HTTP requests")
            .build();

        Self {
            request_duration,
            request_counter,
        }
    }
}

/// Middleware that records HTTP request metrics via OTEL instruments
/// and logs structured tracing fields for trace-level visibility.
pub async fn http_metrics(
    State(metrics): State<HttpMetrics>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let start = Instant::now();

    let response = next.run(request).await;

    let duration_secs = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();
    let method_str = method.to_string();

    let attrs = [
        opentelemetry::KeyValue::new("http.method", method_str.clone()),
        opentelemetry::KeyValue::new("http.route", path.clone()),
        opentelemetry::KeyValue::new("http.status_code", i64::from(status)),
    ];

    metrics.request_duration.record(duration_secs, &attrs);
    metrics.request_counter.add(1, &attrs);

    tracing::info!(
        http.method = %method_str,
        http.route = %path,
        http.status_code = status,
        http.duration_ms = (duration_secs * 1000.0) as u64,
        "request"
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_config_defaults() {
        let config = SecurityHeadersConfig::new("default-src 'self'", 31536000);
        assert_eq!(config.csp_policy, "default-src 'self'");
        assert_eq!(config.hsts_value, "max-age=31536000; includeSubDomains");
    }

    #[test]
    fn security_config_custom() {
        let config = SecurityHeadersConfig::new(
            "default-src 'self'; script-src 'self' cdn.example.com",
            86400,
        );
        assert_eq!(
            config.csp_policy,
            "default-src 'self'; script-src 'self' cdn.example.com"
        );
        assert_eq!(config.hsts_value, "max-age=86400; includeSubDomains");
    }
}
