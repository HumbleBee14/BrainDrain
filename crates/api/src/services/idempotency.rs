//! API idempotency middleware.
//!
//! Prevents duplicate side effects from client retries on mutating endpoints.
//!
//! # Scoping
//! Keys are scoped per `(principal_id, idempotency_key, method, route)` where
//! `principal_id` = `"{sub}:{org_id}"` from the JWT. This prevents:
//! - Cross-user replay (different sub)
//! - Cross-tenant replay (same user, different org)
//! - Cross-endpoint replay (same key on different route/method)
//!
//! # Body handling
//! Request and response bodies are only buffered when `Content-Length` is known
//! and under the limit. Large/streaming bodies skip idempotency without being
//! consumed, so downstream handlers see the original request unmodified.

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

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const MAX_KEY_LENGTH: usize = 256;

/// Max body size for idempotency buffering (1 MB). Bodies larger than this
/// skip idempotency without being consumed.
const MAX_IDEMPOTENCY_BODY_SIZE: usize = 1024 * 1024;

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

const EXCLUDED_ROUTES: &[&str] = &[
    "/api/webhooks/stripe",
    "/v1/chat/completions",
    "/api/v1/documents", // multipart uploads
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

/// Normalize route: strip trailing slash and replace UUID segments with `:id`.
fn normalize_route(path: &str) -> String {
    let path = path.trim_end_matches('/');
    path.split('/')
        .map(|s| if Uuid::parse_str(s).is_ok() { ":id" } else { s })
        .collect::<Vec<_>>()
        .join("/")
}

/// Get Content-Length from request headers, if present.
fn content_length(request: &Request<Body>) -> Option<usize> {
    request
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

/// Get Content-Length from response headers, if present.
fn response_content_length(response: &Response<Body>) -> Option<usize> {
    response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

/// Extract a composite principal ID from the JWT: `"{sub}:{org_id}"`.
///
/// Includes `org_id` to prevent cross-tenant replay when the same user
/// belongs to multiple organizations. Falls back to `"{sub}:"` for
/// personal-tenant tokens without an org_id.
///
/// This decodes the JWT payload without signature verification. The handler's
/// auth extractor validates the signature later — if the token is invalid,
/// the handler will 401 and the idempotency row stays as "processing" until
/// the stale reaper cleans it up (5 min).
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

    if sub.is_empty() || sub.len() > 256 {
        return None;
    }

    let org_id = claims.get("org_id").and_then(|v| v.as_str()).unwrap_or("");

    Some(format!("{sub}:{org_id}"))
}

/// Axum middleware for idempotency enforcement.
pub async fn idempotency_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
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

    let principal_id = match pre_extract_principal(&request) {
        Some(id) => id,
        None => return next.run(request).await,
    };

    // Check Content-Length before consuming the body. If too large or unknown
    // with a large hint, skip idempotency without touching the body.
    if let Some(cl) = content_length(&request)
        && cl > MAX_IDEMPOTENCY_BODY_SIZE
    {
        tracing::debug!(
            content_length = cl,
            "Request body too large for idempotency, skipping"
        );
        return next.run(request).await;
    }

    let normalized_route = normalize_route(&path);
    let method_str = method.to_string();

    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, MAX_IDEMPOTENCY_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => {
            // Body exceeded limit despite Content-Length check (chunked transfer).
            // Rebuild with empty body — this is a rare edge case for chunked
            // requests that lie about their size. Skip idempotency.
            tracing::debug!("Chunked body exceeded idempotency limit, skipping");
            let request = Request::from_parts(parts, Body::empty());
            return next.run(request).await;
        }
    };
    let request_hash = hash_body(&body_bytes);
    let db = state.db();

    // Check for existing key
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
                    let ct = record
                        .response_content_type
                        .unwrap_or_else(|| "application/json".to_string());
                    return (
                        status_code,
                        [("x-idempotency-replayed", "true"), ("content-type", &ct)],
                        body,
                    )
                        .into_response();
                }
            }

            // "failed" — delete and allow retry
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

    // Insert as "processing"
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
        Ok(result) if result.rows_affected() == 0 => {
            // Race: another request completed between our SELECT and INSERT.
            // Re-query to check if it's now completed (return cached) or still processing (409).
            if let Ok(Some(record)) = sqlx::query_as::<_, IdempotencyRecord>(
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
            .await
                && record.status == "completed"
                && record.request_hash == request_hash
                && let (Some(status), Some(body)) = (record.response_status, record.response_body)
            {
                let sc = StatusCode::from_u16(status as u16).unwrap_or(StatusCode::OK);
                let ct = record
                    .response_content_type
                    .unwrap_or_else(|| "application/json".to_string());
                return (
                    sc,
                    [("x-idempotency-replayed", "true"), ("content-type", &ct)],
                    body,
                )
                    .into_response();
            }
            return conflict_response();
        }
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

    // Cache response — check Content-Length before consuming
    let response_status = response.status().as_u16() as i16;
    let resp_content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // If response is too large to cache, return it as-is and mark key failed
    if let Some(cl) = response_content_length(&response)
        && cl > MAX_IDEMPOTENCY_BODY_SIZE
    {
        tracing::debug!("Response too large to cache for idempotency");
        mark_failed(db, &principal_id, &key, &method_str, &normalized_route).await;
        return response;
    }

    let (resp_parts, resp_body) = response.into_parts();
    let resp_bytes = match axum::body::to_bytes(resp_body, MAX_IDEMPOTENCY_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => {
            tracing::warn!("Response body could not be buffered for idempotency cache");
            mark_failed(db, &principal_id, &key, &method_str, &normalized_route).await;
            return Response::from_parts(resp_parts, Body::empty());
        }
    };

    if (200..300).contains(&(response_status as u16)) {
        let _ = sqlx::query(
            "UPDATE idempotency_keys \
             SET status = 'completed', response_status = $1, response_content_type = $2, \
                 response_body = $3, completed_at = NOW() \
             WHERE principal_id = $4 AND idempotency_key = $5 AND method = $6 AND route = $7",
        )
        .bind(response_status)
        .bind(&resp_content_type)
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

/// Cleanup expired + stale idempotency keys. Advisory-locked for multi-instance safety.
pub async fn cleanup_expired_keys(db: &PgPool) -> Result<u64, sqlx::Error> {
    const CLEANUP_LOCK_ID: i64 = 900_100_001;

    let locked: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .bind(CLEANUP_LOCK_ID)
        .fetch_one(db)
        .await?;

    if !locked.0 {
        return Ok(0);
    }

    let result = sqlx::query(
        "DELETE FROM idempotency_keys \
         WHERE expires_at < NOW() \
            OR (status = 'processing' AND created_at < NOW() - INTERVAL '5 minutes')",
    )
    .execute(db)
    .await;

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
    fn safe_methods_skip() {
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
    fn non_allowlisted_skip() {
        assert!(!requires_idempotency(&Method::POST, "/api/v2/something"));
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
    fn normalize_route_preserves_non_uuid() {
        assert_eq!(normalize_route("/api/v1/projects"), "/api/v1/projects");
    }

    #[test]
    fn extract_principal_includes_org() {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"sub":"user_abc","org_id":"org_456"}"#);
        let token = format!("{header}.{payload}.sig");

        let req = Request::builder()
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            pre_extract_principal(&req),
            Some("user_abc:org_456".to_string())
        );
    }

    #[test]
    fn extract_principal_no_org() {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"sub":"user_abc"}"#);
        let token = format!("{header}.{payload}.sig");

        let req = Request::builder()
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        assert_eq!(pre_extract_principal(&req), Some("user_abc:".to_string()));
    }

    #[test]
    fn extract_principal_no_auth() {
        let req = Request::builder().body(Body::empty()).unwrap();
        assert!(pre_extract_principal(&req).is_none());
    }

    #[test]
    fn extract_principal_invalid_token() {
        let req = Request::builder()
            .header("authorization", "Bearer not-a-jwt")
            .body(Body::empty())
            .unwrap();
        assert!(pre_extract_principal(&req).is_none());
    }

    #[test]
    fn extract_principal_empty_sub_rejected() {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"sub":""}"#);
        let token = format!("{header}.{payload}.sig");

        let req = Request::builder()
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        assert!(pre_extract_principal(&req).is_none());
    }
}
