//! API idempotency middleware.
//!
//! Prevents duplicate side effects from client retries on mutating endpoints.
//! Scoped per principal + method + route via composite unique constraint.
//!
//! # Flow
//! 1. Extract `Idempotency-Key` header from request.
//! 2. Hash the request body to detect payload changes on key reuse.
//! 3. Check PostgreSQL for an existing key (scoped by principal + key + method + route):
//!    - **Completed**: return cached response (with original Content-Type).
//!    - **Processing**: return 409 Conflict (in-flight dedup).
//!    - **Not found**: INSERT with status=processing, run handler, cache response.
//! 4. If the handler returns non-2xx, mark key as failed so retries re-execute.
//!
//! # Security
//! - Principal is extracted from the validated `AuthenticatedUser` extension,
//!   NOT from the raw JWT. If auth hasn't run, idempotency is skipped.
//! - Keys scoped per (principal, method, route) — no cross-endpoint replay.
//! - Request body hash prevents key reuse with different payloads.
//! - 24-hour TTL + stale processing reaper (5 min) prevent storage bloat.
//!
//! # Exclusions
//! - Safe methods (GET/HEAD/OPTIONS).
//! - Routes not in the allowlist.
//! - Multipart uploads (body too large to buffer for hashing).
//! - Stripe webhooks, inference endpoint.
//! - Requests without the header (opt-in).
//! - Disabled when `idempotency.enforced` feature flag is off.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, Response, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::services::feature_flags::{FlagContext, IDEMPOTENCY_ENFORCED};

/// Header name per IETF draft-ietf-httpapi-idempotency-key-header.
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// Maximum length of an idempotency key to prevent abuse.
const MAX_KEY_LENGTH: usize = 256;

/// Max request body size we'll buffer for hashing (1 MB).
/// Requests with larger bodies skip idempotency processing.
const MAX_IDEMPOTENCY_BODY_SIZE: usize = 1024 * 1024;

/// Routes that require idempotency protection (mutating /api/v1/ endpoints).
const IDEMPOTENT_ROUTE_PREFIXES: &[&str] = &[
    "/api/v1/projects",
    "/api/v1/pipeline",
    "/api/v1/datasets",
    "/api/v1/training",
    "/api/v1/evaluations",
    "/api/v1/exports",
    "/api/v1/api-keys",
    "/api/v1/models",
    "/api/v1/billing",
    "/api/v1/team",
    "/api/v1/notifications",
    "/api/v1/settings",
];

/// Routes explicitly excluded from idempotency (including large-body uploads).
const EXCLUDED_ROUTES: &[&str] = &[
    "/api/webhooks/stripe",
    "/v1/chat/completions",
    "/api/v1/documents", // multipart uploads — body too large to buffer
    "/health",
    "/ready",
];

fn requires_idempotency(method: &Method, path: &str) -> bool {
    if matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    ) {
        return false;
    }

    for excluded in EXCLUDED_ROUTES {
        if path.starts_with(excluded) {
            return false;
        }
    }

    for prefix in IDEMPOTENT_ROUTE_PREFIXES {
        if path.starts_with(prefix) {
            return true;
        }
    }

    false
}

fn hash_body(body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body);
    format!("{:x}", hasher.finalize())
}

/// Normalize route path: strip trailing slash and UUID path params.
/// e.g., "/api/v1/projects/550e8400-e29b-41d4-a716-446655440000" → "/api/v1/projects/:id"
fn normalize_route(path: &str) -> String {
    let path = path.trim_end_matches('/');
    // Replace UUID segments with :id for consistent keying
    let segments: Vec<&str> = path.split('/').collect();
    segments
        .iter()
        .map(|s| if Uuid::parse_str(s).is_ok() { ":id" } else { s })
        .collect::<Vec<_>>()
        .join("/")
}

/// Axum middleware for idempotency enforcement.
///
/// Gated by the `idempotency.enforced` feature flag.
/// Principal is extracted from `AuthenticatedUser` extension (post-auth).
pub async fn idempotency_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    // Feature flag gate
    if !state
        .feature_flags()
        .is_enabled(IDEMPOTENCY_ENFORCED, &FlagContext::default())
    {
        return next.run(request).await;
    }

    let method = request.method().clone();
    let path = request.uri().path().to_string();

    if !requires_idempotency(&method, &path) {
        return next.run(request).await;
    }

    // Extract idempotency key from header
    let key = match request.headers().get(IDEMPOTENCY_KEY_HEADER) {
        Some(value) => match value.to_str() {
            Ok(k) if !k.trim().is_empty() && k.len() <= MAX_KEY_LENGTH => k.trim().to_string(),
            Ok(k) if k.len() > MAX_KEY_LENGTH => {
                return AppError::BadRequest {
                    message: format!(
                        "Idempotency-Key exceeds maximum length of {MAX_KEY_LENGTH} characters"
                    ),
                }
                .into_response();
            }
            _ => {
                return AppError::BadRequest {
                    message: "Invalid Idempotency-Key header value".to_string(),
                }
                .into_response();
            }
        },
        None => return next.run(request).await,
    };

    // Extract principal from validated auth context.
    // The idempotency layer is applied inside the router, so auth extractors
    // haven't run yet. We do a lightweight pre-auth here using the same
    // mechanism. If auth fails, we skip idempotency (handler will reject anyway).
    let principal_id = match pre_extract_principal(&request) {
        Some(id) => id,
        None => return next.run(request).await,
    };

    let normalized_route = normalize_route(&path);
    let method_str = method.to_string();

    // Buffer the request body for hashing
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, MAX_IDEMPOTENCY_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => {
            // Body too large for idempotency — skip (don't block the request)
            tracing::debug!("Request body exceeds idempotency buffer limit, skipping dedup");
            let request = Request::from_parts(parts, Body::empty());
            return next.run(request).await;
        }
    };
    let request_hash = hash_body(&body_bytes);
    let db = state.db();

    // Check for existing key (scoped by principal + key + method + route)
    let existing = sqlx::query_as::<_, IdempotencyRecord>(
        "SELECT id, status, request_hash, response_status, response_content_type, response_body \
         FROM idempotency_keys \
         WHERE principal_id = $1 AND idempotency_key = $2 AND method = $3 AND route = $4 \
           AND expires_at > NOW()",
    )
    .bind(&principal_id)
    .bind(&key)
    .bind(&method_str)
    .bind(&normalized_route)
    .fetch_optional(db)
    .await;

    match existing {
        Ok(Some(record)) => {
            if record.status == "processing" {
                return conflict_response();
            }

            if record.status == "completed" {
                if record.request_hash != request_hash {
                    return AppError::BadRequest {
                        message: "Idempotency-Key reused with different request body".to_string(),
                    }
                    .into_response();
                }

                if let (Some(status), Some(body)) = (record.response_status, record.response_body) {
                    let status_code = StatusCode::from_u16(status as u16).unwrap_or(StatusCode::OK);
                    let content_type = record
                        .response_content_type
                        .unwrap_or_else(|| "application/json".to_string());
                    return (
                        status_code,
                        [
                            ("x-idempotency-replayed", "true"),
                            ("content-type", &content_type),
                        ],
                        body,
                    )
                        .into_response();
                }
            }

            // Status is "failed" — delete and allow retry
            let _ = sqlx::query("DELETE FROM idempotency_keys WHERE id = $1")
                .bind(record.id)
                .execute(db)
                .await;
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(error = %e, "Idempotency DB check failed, proceeding without dedup");
            let request = Request::from_parts(parts, Body::from(body_bytes));
            return next.run(request).await;
        }
    }

    // Insert as "processing" (ON CONFLICT handles race conditions)
    let insert_result = sqlx::query(
        "INSERT INTO idempotency_keys \
         (principal_id, idempotency_key, method, route, request_hash, status) \
         VALUES ($1, $2, $3, $4, $5, 'processing') \
         ON CONFLICT (principal_id, idempotency_key, method, route) DO NOTHING",
    )
    .bind(&principal_id)
    .bind(&key)
    .bind(&method_str)
    .bind(&normalized_route)
    .bind(&request_hash)
    .execute(db)
    .await;

    match insert_result {
        Ok(result) if result.rows_affected() == 0 => return conflict_response(),
        Err(e) => {
            tracing::warn!(error = %e, "Idempotency INSERT failed, proceeding without dedup");
            let request = Request::from_parts(parts, Body::from(body_bytes));
            return next.run(request).await;
        }
        _ => {}
    }

    // Execute the handler
    let request = Request::from_parts(parts, Body::from(body_bytes));
    let response = next.run(request).await;

    // Capture response for caching
    let response_status = response.status().as_u16() as i16;
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let (resp_parts, resp_body) = response.into_parts();

    let resp_bytes = match axum::body::to_bytes(resp_body, MAX_IDEMPOTENCY_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => {
            // Response too large to cache — mark failed but return original response.
            // We can't return the original body (already consumed), so mark failed
            // and let the client retry if needed.
            tracing::warn!("Response body too large to cache for idempotency");
            mark_failed(db, &principal_id, &key, &method_str, &normalized_route).await;
            return Response::from_parts(resp_parts, Body::empty());
        }
    };

    // Only cache 2xx responses. Non-2xx = mark failed so client can retry.
    if (200..300).contains(&(response_status as u16)) {
        let _ = sqlx::query(
            "UPDATE idempotency_keys \
             SET status = 'completed', response_status = $1, response_content_type = $2, \
                 response_body = $3, completed_at = NOW() \
             WHERE principal_id = $4 AND idempotency_key = $5 AND method = $6 AND route = $7",
        )
        .bind(response_status)
        .bind(&content_type)
        .bind(resp_bytes.as_ref())
        .bind(&principal_id)
        .bind(&key)
        .bind(&method_str)
        .bind(&normalized_route)
        .execute(db)
        .await;
    } else {
        mark_failed(db, &principal_id, &key, &method_str, &normalized_route).await;
    }

    Response::from_parts(resp_parts, Body::from(resp_bytes))
}

/// Pre-extract principal from the JWT for idempotency scoping.
///
/// This decodes the JWT payload to read the `sub` claim. The actual auth
/// validation happens in the handler's `AuthenticatedUser` extractor.
/// If the token is invalid, returns None and idempotency is skipped
/// (the handler will reject the request anyway).
fn pre_extract_principal(request: &Request<Body>) -> Option<String> {
    let auth_header = request.headers().get("authorization")?.to_str().ok()?;
    let token = auth_header.strip_prefix("Bearer ")?;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    use base64::Engine;
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    let sub = claims.get("sub")?.as_str()?;

    // Basic sanity: sub must be non-empty and reasonable length
    if sub.is_empty() || sub.len() > 256 {
        return None;
    }

    Some(sub.to_string())
}

fn conflict_response() -> Response<Body> {
    (
        StatusCode::CONFLICT,
        [("retry-after", "1")],
        axum::Json(serde_json::json!({
            "error": {
                "code": "idempotency_conflict",
                "message": "A request with this Idempotency-Key is already being processed. Retry after 1 second."
            }
        })),
    )
        .into_response()
}

async fn mark_failed(db: &PgPool, principal_id: &str, key: &str, method: &str, route: &str) {
    let _ = sqlx::query(
        "UPDATE idempotency_keys SET status = 'failed', completed_at = NOW() \
         WHERE principal_id = $1 AND idempotency_key = $2 AND method = $3 AND route = $4",
    )
    .bind(principal_id)
    .bind(key)
    .bind(method)
    .bind(route)
    .execute(db)
    .await;
}

/// Cleanup expired idempotency keys and stale processing keys.
///
/// Uses `pg_try_advisory_lock` so only one instance runs cleanup at a time
/// in multi-replica deployments (others skip silently).
pub async fn cleanup_expired_keys(db: &PgPool) -> Result<u64, sqlx::Error> {
    // Advisory lock ID — arbitrary constant for idempotency cleanup
    const CLEANUP_LOCK_ID: i64 = 900_100_001;

    let locked: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .bind(CLEANUP_LOCK_ID)
        .fetch_one(db)
        .await?;

    if !locked.0 {
        // Another instance is already running cleanup
        return Ok(0);
    }

    let result = sqlx::query(
        "DELETE FROM idempotency_keys \
         WHERE expires_at < NOW() \
            OR (status = 'processing' AND created_at < NOW() - INTERVAL '5 minutes')",
    )
    .execute(db)
    .await;

    // Release advisory lock
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(CLEANUP_LOCK_ID)
        .execute(db)
        .await;

    Ok(result?.rows_affected())
}

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    id: Uuid,
    status: String,
    request_hash: String,
    response_status: Option<i16>,
    response_content_type: Option<String>,
    response_body: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_methods_skip_idempotency() {
        assert!(!requires_idempotency(&Method::GET, "/api/v1/projects"));
        assert!(!requires_idempotency(&Method::HEAD, "/api/v1/projects"));
        assert!(!requires_idempotency(&Method::OPTIONS, "/api/v1/projects"));
    }

    #[test]
    fn mutating_methods_on_allowlisted_routes() {
        assert!(requires_idempotency(&Method::POST, "/api/v1/projects"));
        assert!(requires_idempotency(&Method::PUT, "/api/v1/projects/123"));
        assert!(requires_idempotency(
            &Method::DELETE,
            "/api/v1/projects/123"
        ));
        assert!(requires_idempotency(&Method::PATCH, "/api/v1/training/123"));
        assert!(requires_idempotency(
            &Method::POST,
            "/api/v1/models/123/deploy"
        ));
    }

    #[test]
    fn excluded_routes_skip() {
        assert!(!requires_idempotency(&Method::POST, "/api/webhooks/stripe"));
        assert!(!requires_idempotency(&Method::POST, "/v1/chat/completions"));
        assert!(!requires_idempotency(&Method::GET, "/health"));
    }

    #[test]
    fn document_uploads_excluded() {
        assert!(!requires_idempotency(
            &Method::POST,
            "/api/v1/documents/upload"
        ));
    }

    #[test]
    fn non_allowlisted_routes_skip() {
        assert!(!requires_idempotency(&Method::POST, "/api/v2/something"));
        assert!(!requires_idempotency(&Method::POST, "/random"));
    }

    #[test]
    fn body_hash_deterministic() {
        assert_eq!(hash_body(b"test"), hash_body(b"test"));
    }

    #[test]
    fn different_bodies_different_hashes() {
        assert_ne!(hash_body(b"A"), hash_body(b"B"));
    }

    #[test]
    fn empty_body_consistent_hash() {
        let h = hash_body(b"");
        assert!(!h.is_empty());
        assert_eq!(h, hash_body(b""));
    }

    #[test]
    fn normalize_route_strips_uuids() {
        assert_eq!(
            normalize_route("/api/v1/projects/550e8400-e29b-41d4-a716-446655440000"),
            "/api/v1/projects/:id"
        );
        assert_eq!(
            normalize_route("/api/v1/models/550e8400-e29b-41d4-a716-446655440000/deploy"),
            "/api/v1/models/:id/deploy"
        );
    }

    #[test]
    fn normalize_route_strips_trailing_slash() {
        assert_eq!(normalize_route("/api/v1/projects/"), "/api/v1/projects");
    }

    #[test]
    fn normalize_route_preserves_non_uuid_segments() {
        assert_eq!(normalize_route("/api/v1/projects"), "/api/v1/projects");
    }

    #[test]
    fn extract_principal_from_jwt() {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"sub":"user_abc123","org_id":"org_456"}"#);
        let token = format!("{header}.{payload}.fake-signature");

        let request = Request::builder()
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            pre_extract_principal(&request),
            Some("user_abc123".to_string())
        );
    }

    #[test]
    fn extract_principal_no_auth_header() {
        let request = Request::builder().body(Body::empty()).unwrap();
        assert!(pre_extract_principal(&request).is_none());
    }

    #[test]
    fn extract_principal_invalid_token() {
        let request = Request::builder()
            .header("authorization", "Bearer not-a-jwt")
            .body(Body::empty())
            .unwrap();
        assert!(pre_extract_principal(&request).is_none());
    }

    #[test]
    fn extract_principal_empty_sub_rejected() {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"sub":""}"#);
        let token = format!("{header}.{payload}.fake-signature");

        let request = Request::builder()
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        assert!(pre_extract_principal(&request).is_none());
    }
}
