use std::net::SocketAddr;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::header::{
    CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS,
    X_FRAME_OPTIONS,
};
use axum::http::{HeaderName, HeaderValue, Request, Response};
use axum::middleware::Next;
use axum::response::IntoResponse;
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

// -- IP Rate Limiting --

/// Configuration for IP-based rate limiting.
/// Constructed once at startup, passed to the middleware via State.
#[derive(Clone)]
pub struct IpRateLimiter {
    redis: redis::aio::ConnectionManager,
    rpm: u32,
    enabled: bool,
}

impl IpRateLimiter {
    pub fn new(redis: redis::aio::ConnectionManager, config: &Config) -> Self {
        Self {
            redis,
            rpm: config.rate_limit_rpm,
            enabled: config.rate_limit_enabled,
        }
    }
}

/// Extract the client IP address from the request.
///
/// Priority: X-Forwarded-For (first IP) > X-Real-IP > peer socket address > "unknown".
fn extract_client_ip(request: &Request<Body>) -> String {
    // X-Forwarded-For: client, proxy1, proxy2
    if let Some(xff) = request.headers().get("x-forwarded-for")
        && let Ok(value) = xff.to_str()
        && let Some(first_ip) = value.split(',').next()
    {
        let ip = first_ip.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }

    // X-Real-IP (single IP, set by nginx)
    if let Some(real_ip) = request.headers().get("x-real-ip")
        && let Ok(value) = real_ip.to_str()
    {
        let ip = value.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }

    // Fallback: peer socket address from ConnectInfo
    if let Some(connect_info) = request.extensions().get::<ConnectInfo<SocketAddr>>() {
        return connect_info.0.ip().to_string();
    }

    "unknown".to_string()
}

/// Middleware that enforces per-IP rate limiting using Redis.
///
/// Uses the same INCR + EXPIRE sliding window pattern as API key rate limiting.
/// Best-effort: if Redis is unreachable, the request is allowed through.
pub async fn ip_rate_limit(
    State(limiter): State<IpRateLimiter>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    if !limiter.enabled {
        return next.run(request).await;
    }

    let client_ip = extract_client_ip(&request);

    // Build Redis key: ip_rl:{ip}:{YYYYMMDDHHMM}
    let minute = chrono::Utc::now().format("%Y%m%d%H%M").to_string();
    let redis_key = format!(
        "{}{}:{}",
        platform_shared::constants::REDIS_IP_RATE_LIMIT_PREFIX,
        client_ip,
        minute,
    );

    // Best-effort: if Redis fails, allow the request
    let mut redis = limiter.redis.clone();
    let count: i64 = match redis::cmd("INCR")
        .arg(&redis_key)
        .query_async::<i64>(&mut redis)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "IP rate limiter: Redis INCR failed, allowing request");
            return next.run(request).await;
        }
    };

    // Set TTL on first request in this window
    if count == 1 {
        let _: Result<(), redis::RedisError> = redis::cmd("EXPIRE")
            .arg(&redis_key)
            .arg(60)
            .query_async(&mut redis)
            .await;
    }

    let remaining = (limiter.rpm as i64 - count).max(0);

    if count > limiter.rpm as i64 {
        tracing::warn!(
            client_ip = %client_ip,
            count = count,
            limit = limiter.rpm,
            "IP rate limit exceeded"
        );

        let mut response = crate::error::AppError::RateLimited.into_response();
        let headers = response.headers_mut();
        headers.insert(
            HeaderName::from_static("retry-after"),
            HeaderValue::from_static("60"),
        );
        headers.insert(
            HeaderName::from_static("x-ratelimit-limit"),
            HeaderValue::from(limiter.rpm),
        );
        headers.insert(
            HeaderName::from_static("x-ratelimit-remaining"),
            HeaderValue::from(0u32),
        );
        return response;
    }

    // Run the request, then attach rate limit headers to the response
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        HeaderName::from_static("x-ratelimit-limit"),
        HeaderValue::from(limiter.rpm),
    );
    headers.insert(
        HeaderName::from_static("x-ratelimit-remaining"),
        HeaderValue::from(remaining as u32),
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

    // -- IP extraction tests --

    #[test]
    fn extract_ip_from_xff_header() {
        let req = Request::builder()
            .header("x-forwarded-for", "203.0.113.50, 70.41.3.18, 150.172.238.178")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_client_ip(&req), "203.0.113.50");
    }

    #[test]
    fn extract_ip_from_x_real_ip() {
        let req = Request::builder()
            .header("x-real-ip", "10.0.0.1")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_client_ip(&req), "10.0.0.1");
    }

    #[test]
    fn extract_ip_xff_takes_priority() {
        let req = Request::builder()
            .header("x-forwarded-for", "1.2.3.4")
            .header("x-real-ip", "5.6.7.8")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_client_ip(&req), "1.2.3.4");
    }

    #[test]
    fn extract_ip_fallback_to_unknown() {
        let req = Request::builder()
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_client_ip(&req), "unknown");
    }
}
