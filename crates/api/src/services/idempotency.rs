//! API idempotency middleware.
//!
//! Prevents duplicate side effects from client retries on mutating endpoints.
//! Scoped per principal (JWT `sub` claim) via composite unique constraint.
//!
//! # Flow
//! 1. Extract `Idempotency-Key` header from request.
//! 2. Hash the request body to detect payload changes on key reuse.
//! 3. Check PostgreSQL for an existing key:
//!    - **Completed**: return cached response immediately.
//!    - **Processing**: return 409 Conflict (in-flight dedup).
//!    - **Not found**: INSERT with status=processing, run handler, cache response.
//! 4. If the handler returns non-2xx, mark key as failed so retries re-execute.
//!
//! # Exclusions
//! - GET/HEAD/OPTIONS requests (safe methods).
//! - Routes not in the idempotency allowlist.
//! - Stripe webhook endpoint (has its own dedup via event IDs).
//! - OpenAI-compatible inference endpoint (stateless, high throughput).
//! - Requests without the header (opt-in, not mandatory).
//!
//! # Security
//! - Keys scoped per JWT `sub` — one user cannot replay another's key.
//! - Request body hash prevents key reuse with different payloads.
//! - 24-hour TTL prevents unbounded storage growth.
//! - Max key length enforced to prevent abuse.

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

/// Header name per IETF draft-ietf-httpapi-idempotency-key-header.
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// Maximum length of an idempotency key to prevent abuse.
const MAX_KEY_LENGTH: usize = 256;

/// Max request body size we'll buffer for hashing (10 MB).
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Routes that require idempotency protection (mutating /api/v1/ endpoints).
const IDEMPOTENT_ROUTE_PREFIXES: &[&str] = &[
    "/api/v1/projects",
    "/api/v1/documents",
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

/// Routes explicitly excluded from idempotency enforcement.
const EXCLUDED_ROUTES: &[&str] = &[
    "/api/webhooks/stripe",
    "/v1/chat/completions",
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

/// Extract the JWT `sub` claim from the Authorization header.
///
/// Decodes the JWT payload without signature verification — only reads the
/// `sub` claim for idempotency scoping. The handler's auth extractor does
/// full validation later. Returns None for unauthenticated requests.
fn extract_principal(request: &Request<Body>) -> Option<String> {
    let auth_header = request.headers().get("authorization")?.to_str().ok()?;
    let token = auth_header.strip_prefix("Bearer ")?;

    // JWT is base64(header).base64(payload).signature — decode payload directly.
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    use base64::Engine;
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    claims.get("sub")?.as_str().map(|s| s.to_string())
}

/// Axum middleware for idempotency enforcement.
pub async fn idempotency_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
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

    // Extract principal from JWT (no DB call, no signature check)
    let principal_id = match extract_principal(&request) {
        Some(id) => id,
        None => return next.run(request).await,
    };

    // Buffer the request body for hashing
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, MAX_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => {
            return AppError::BadRequest {
                message: "Request body too large for idempotency processing".to_string(),
            }
            .into_response();
        }
    };
    let request_hash = hash_body(&body_bytes);
    let db = state.db();

    // Check for existing key
    let existing = sqlx::query_as::<_, IdempotencyRecord>(
        "SELECT id, status, request_hash, response_status, response_body \
         FROM idempotency_keys \
         WHERE principal_id = $1 AND idempotency_key = $2 AND expires_at > NOW()",
    )
    .bind(&principal_id)
    .bind(&key)
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
                    return (status_code, [("x-idempotency-replayed", "true")], body)
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
         ON CONFLICT (principal_id, idempotency_key) DO NOTHING",
    )
    .bind(&principal_id)
    .bind(&key)
    .bind(method.as_str())
    .bind(&path)
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
    let (resp_parts, resp_body) = response.into_parts();

    let resp_bytes = match axum::body::to_bytes(resp_body, MAX_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => {
            mark_failed(db, &principal_id, &key).await;
            return Response::from_parts(resp_parts, Body::empty());
        }
    };

    // Only cache 2xx responses. Non-2xx = mark failed so client can retry.
    if (200..300).contains(&(response_status as u16)) {
        let _ = sqlx::query(
            "UPDATE idempotency_keys \
             SET status = 'completed', response_status = $1, response_body = $2, completed_at = NOW() \
             WHERE principal_id = $3 AND idempotency_key = $4",
        )
        .bind(response_status)
        .bind(resp_bytes.as_ref())
        .bind(&principal_id)
        .bind(&key)
        .execute(db)
        .await;
    } else {
        mark_failed(db, &principal_id, &key).await;
    }

    Response::from_parts(resp_parts, Body::from(resp_bytes))
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

async fn mark_failed(db: &PgPool, principal_id: &str, key: &str) {
    let _ = sqlx::query(
        "UPDATE idempotency_keys SET status = 'failed', completed_at = NOW() \
         WHERE principal_id = $1 AND idempotency_key = $2",
    )
    .bind(principal_id)
    .bind(key)
    .execute(db)
    .await;
}

/// Cleanup expired idempotency keys. Call periodically (e.g., hourly).
pub async fn cleanup_expired_keys(db: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM idempotency_keys WHERE expires_at < NOW() AND status != 'processing'",
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    id: Uuid,
    status: String,
    request_hash: String,
    response_status: Option<i16>,
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
    fn extract_principal_from_jwt() {
        // Build a minimal JWT: header.payload.signature
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

        let principal = extract_principal(&request);
        assert_eq!(principal, Some("user_abc123".to_string()));
    }

    #[test]
    fn extract_principal_no_auth_header() {
        let request = Request::builder().body(Body::empty()).unwrap();
        assert!(extract_principal(&request).is_none());
    }

    #[test]
    fn extract_principal_invalid_token() {
        let request = Request::builder()
            .header("authorization", "Bearer not-a-jwt")
            .body(Body::empty())
            .unwrap();
        assert!(extract_principal(&request).is_none());
    }
}
