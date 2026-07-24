use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use ipnet::IpNet;

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
            axum::http::HeaderName::from_static("idempotency-key"),
        ])
        .expose_headers([
            axum::http::HeaderName::from_static("x-idempotency-replayed"),
            axum::http::HeaderName::from_static("x-request-id"),
            axum::http::HeaderName::from_static("x-ratelimit-limit"),
            axum::http::HeaderName::from_static("x-ratelimit-remaining"),
            axum::http::HeaderName::from_static("retry-after"),
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

/// Span for each traced request: method + path, never the query string —
/// query params can carry credentials (WebSocket auth tokens, presigned URLs)
/// and must not reach logs.
fn request_span(request: &Request<Body>) -> tracing::Span {
    tracing::info_span!(
        "request",
        method = %request.method(),
        path = %request.uri().path(),
    )
}

type RequestSpanFn = fn(&Request<Body>) -> tracing::Span;

/// HTTP request/response tracing layer.
pub fn trace_layer() -> TraceLayer<
    tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>,
    RequestSpanFn,
> {
    TraceLayer::new_for_http().make_span_with(request_span as RequestSpanFn)
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
    trusted_proxies: Arc<[IpNet]>,
}

impl IpRateLimiter {
    pub fn new(redis: redis::aio::ConnectionManager, config: &Config) -> Self {
        Self {
            redis,
            rpm: config.rate_limit_rpm,
            enabled: config.rate_limit_enabled,
            trusted_proxies: parse_trusted_proxies(&config.trusted_proxy_cidrs_list()),
        }
    }
}

/// Parse CIDR strings (bare IPs allowed as /32 or /128) into networks.
/// Invalid entries are dropped with a warning — failing closed: an unparsed
/// entry means less trust, never more.
fn parse_trusted_proxies(cidrs: &[String]) -> Arc<[IpNet]> {
    cidrs
        .iter()
        .filter_map(|s| {
            s.parse::<IpNet>()
                .or_else(|_| s.parse::<IpAddr>().map(IpNet::from))
                .map_err(|_| {
                    tracing::warn!(entry = %s, "TRUSTED_PROXY_CIDRS: invalid entry ignored");
                })
                .ok()
        })
        .collect()
}

fn is_trusted_proxy(ip: IpAddr, trusted: &[IpNet]) -> bool {
    trusted.iter().any(|net| net.contains(&ip))
}

/// Extract the client IP address used as the rate-limit bucket key.
///
/// - Socket IP not in a trusted proxy CIDR (or no proxies configured): use the
///   socket IP and ignore forwarded headers entirely — spoof-proof.
/// - Socket IP is a trusted proxy: scan X-Forwarded-For right-to-left and take
///   the first entry that is not itself a trusted proxy (rightmost-untrusted;
///   the leftmost values are client-controlled and never trusted blindly).
///   Missing/all-trusted XFF falls back to the socket IP.
/// - No socket info at all: "unknown" (shared bucket, fail-safe).
fn extract_client_ip(request: &Request<Body>, trusted_proxies: &[IpNet]) -> String {
    let Some(connect_info) = request.extensions().get::<ConnectInfo<SocketAddr>>() else {
        return "unknown".to_string();
    };
    let socket_ip = connect_info.0.ip();

    if !is_trusted_proxy(socket_ip, trusted_proxies) {
        return socket_ip.to_string();
    }

    if let Some(xff) = request.headers().get("x-forwarded-for")
        && let Ok(value) = xff.to_str()
    {
        for entry in value.rsplit(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            match entry.parse::<IpAddr>() {
                Ok(ip) if is_trusted_proxy(ip, trusted_proxies) => continue,
                Ok(ip) => return ip.to_string(),
                // Non-IP entry appended by a trusted proxy: keep it as the
                // bucket key rather than trusting anything further left.
                Err(_) => return entry.to_string(),
            }
        }
    }

    socket_ip.to_string()
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

    let client_ip = extract_client_ip(&request, &limiter.trusted_proxies);

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

    fn proxies(cidrs: &[&str]) -> Vec<IpNet> {
        parse_trusted_proxies(&cidrs.iter().map(|s| s.to_string()).collect::<Vec<_>>()).to_vec()
    }

    fn request_from(socket_ip: &str, xff: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().extension(ConnectInfo(SocketAddr::new(
            socket_ip.parse().unwrap(),
            12345,
        )));
        if let Some(xff) = xff {
            builder = builder.header("x-forwarded-for", xff);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn direct_client_uses_socket_ip_ignoring_spoofed_xff() {
        // No trusted proxies configured: XFF is attacker-controlled, ignore it.
        let req = request_from("203.0.113.50", Some("1.2.3.4"));
        assert_eq!(extract_client_ip(&req, &[]), "203.0.113.50");
    }

    #[test]
    fn untrusted_socket_ignores_xff_even_with_proxies_configured() {
        let trusted = proxies(&["10.0.0.0/8"]);
        let req = request_from("203.0.113.50", Some("1.2.3.4"));
        assert_eq!(extract_client_ip(&req, &trusted), "203.0.113.50");
    }

    #[test]
    fn trusted_proxy_resolves_rightmost_untrusted() {
        let trusted = proxies(&["10.0.0.0/8"]);
        // Spoofed leftmost entry must not win; the rightmost untrusted entry
        // (appended by the proxy) is the real client.
        let req = request_from("10.0.0.1", Some("1.2.3.4, 198.51.100.7"));
        assert_eq!(extract_client_ip(&req, &trusted), "198.51.100.7");
    }

    #[test]
    fn trusted_proxy_chain_skips_trusted_hops() {
        let trusted = proxies(&["10.0.0.0/8", "192.168.1.1"]);
        let req = request_from("10.0.0.1", Some("198.51.100.7, 192.168.1.1, 10.0.0.2"));
        assert_eq!(extract_client_ip(&req, &trusted), "198.51.100.7");
    }

    #[test]
    fn trusted_proxy_without_xff_falls_back_to_socket_ip() {
        let trusted = proxies(&["10.0.0.0/8"]);
        assert_eq!(
            extract_client_ip(&request_from("10.0.0.1", None), &trusted),
            "10.0.0.1"
        );
        assert_eq!(
            extract_client_ip(&request_from("10.0.0.1", Some("")), &trusted),
            "10.0.0.1"
        );
    }

    #[test]
    fn all_trusted_xff_falls_back_to_socket_ip() {
        let trusted = proxies(&["10.0.0.0/8"]);
        let req = request_from("10.0.0.1", Some("10.0.0.2, 10.0.0.3"));
        assert_eq!(extract_client_ip(&req, &trusted), "10.0.0.1");
    }

    #[test]
    fn no_connect_info_is_unknown() {
        let req = Request::builder()
            .header("x-forwarded-for", "1.2.3.4")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_client_ip(&req, &[]), "unknown");
    }

    #[test]
    fn invalid_cidr_entries_dropped() {
        let trusted = proxies(&["not-a-cidr", "10.0.0.0/8"]);
        assert_eq!(trusted.len(), 1);
    }
}
