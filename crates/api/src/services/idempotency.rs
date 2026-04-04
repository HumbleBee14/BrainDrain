//! API idempotency middleware.
//!
//! Prevents duplicate side effects from client retries on mutating endpoints.
//!
//! # Scoping
//! Keys are scoped per `(principal_id, idempotency_key, method, route)` where
//! `principal_id` = `"{user_id}:{tenant_id}"` from a fully verified auth context.
//! This prevents cross-user, cross-tenant, and cross-endpoint replay.
//!
//! # Auth verification
//! The middleware calls `auth_chain.authenticate()` to verify the JWT signature
//! and resolve the tenant BEFORE writing any idempotency rows. Forged tokens
//! are rejected — no rows are created for unauthenticated requests.
//!
//! # Body handling
//! Request and response bodies are only buffered when `Content-Length` is present
//! and within the 1 MB limit. If `Content-Length` is missing (chunked/streaming)
//! or exceeds the limit, idempotency is skipped entirely — the body stream is
//! never consumed, and the handler/client sees the original data unmodified.

use axum::body::Body;
use axum::extract::State;
use axum::http::header::CONTENT_LENGTH;
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
const MAX_IDEMPOTENCY_BODY_SIZE: usize = 1024 * 1024; // 1 MB

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
    "/api/v1/invitations",
];

const EXCLUDED_ROUTES: &[&str] = &[
    "/api/webhooks/stripe",
    "/v1/chat/completions",
    "/health",
    "/ready",
];

/// Path segments that indicate a multipart upload route.
/// These are excluded because buffering large file uploads is not feasible.
const UPLOAD_PATH_SEGMENTS: &[&str] = &["/documents"];

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

    // Exclude upload routes: /api/v1/projects/{id}/documents
    if method == Method::POST {
        for segment in UPLOAD_PATH_SEGMENTS {
            if path.contains(segment) {
                return false;
            }
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

fn normalize_route(path: &str) -> String {
    let path = path.trim_end_matches('/');
    path.split('/')
        .map(|s| if Uuid::parse_str(s).is_ok() { ":id" } else { s })
        .collect::<Vec<_>>()
        .join("/")
}

/// Get request Content-Length if present. Returns None for chunked/streaming.
fn request_content_length(request: &Request<Body>) -> Option<usize> {
    request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

/// Get response Content-Length if present.
fn response_content_length(response: &Response<Body>) -> Option<usize> {
    response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

/// Axum middleware for idempotency enforcement.
///
/// Uses `auth_chain.authenticate()` for fully verified JWT before writing rows.
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

    // Fully verify the JWT and resolve tenant BEFORE writing any rows.
    // This prevents forged tokens from creating idempotency rows.
    let token = match request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    {
        Some(t) => t.to_string(),
        None => return next.run(request).await,
    };

    let auth_user = match state.auth_chain().authenticate(&token, state.db()).await {
        Ok(user) => user,
        Err(_) => {
            // Auth failed — skip idempotency, let handler return 401 naturally.
            return next.run(request).await;
        }
    };

    let principal_id = format!("{}:{}", auth_user.user_id, auth_user.tenant_id);

    // Only buffer the body if Content-Length is known and fits within the limit.
    // If Content-Length is missing (chunked/streaming) or too large, skip
    // idempotency entirely WITHOUT consuming the body stream.
    let req_cl = match request_content_length(&request) {
        Some(cl) if cl <= MAX_IDEMPOTENCY_BODY_SIZE => cl,
        Some(_) => {
            tracing::debug!("Request body too large for idempotency, skipping");
            return next.run(request).await;
        }
        None => {
            // No Content-Length = chunked/streaming. Cannot safely buffer without
            // risking body consumption. Skip idempotency transparently.
            tracing::debug!("Request has no Content-Length, skipping idempotency");
            return next.run(request).await;
        }
    };

    let normalized_route = normalize_route(&path);
    let method_str = method.to_string();

    // Safe to buffer: Content-Length confirmed it fits.
    let (parts, body) = request.into_parts();
    let body_bytes = match axum::body::to_bytes(body, req_cl + 1).await {
        Ok(b) => b,
        Err(_) => {
            // Content-Length lied. This shouldn't happen with well-behaved clients.
            // Body is consumed — nothing we can do. Log and pass empty body.
            tracing::error!("Content-Length mismatch: body larger than declared");
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
            // Race: re-query to check if now completed
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

    // Cache response — only buffer if Content-Length is known and fits.
    // If unknown or too large, return response as-is (no modification).
    let response_status = response.status().as_u16() as i16;
    let resp_content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let resp_cl = response_content_length(&response);
    if resp_cl.is_none() || resp_cl.is_some_and(|cl| cl > MAX_IDEMPOTENCY_BODY_SIZE) {
        // Can't safely buffer — return response untouched, mark key failed for retry.
        if (200..300).contains(&(response_status as u16)) {
            tracing::debug!("Response body too large or unknown size, skipping idempotency cache");
        }
        mark_failed(db, &principal_id, &key, &method_str, &normalized_route).await;
        return response;
    }

    let resp_limit = resp_cl.unwrap() + 1;
    let (resp_parts, resp_body) = response.into_parts();
    let resp_bytes = match axum::body::to_bytes(resp_body, resp_limit).await {
        Ok(b) => b,
        Err(_) => {
            tracing::error!("Response Content-Length mismatch during idempotency caching");
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
    fn document_upload_routes_excluded() {
        // The actual upload route: /api/v1/projects/{id}/documents
        assert!(!requires_idempotency(
            &Method::POST,
            "/api/v1/projects/550e8400-e29b-41d4-a716-446655440000/documents"
        ));
        // GET documents is already excluded by safe method check
        assert!(!requires_idempotency(
            &Method::GET,
            "/api/v1/projects/123/documents"
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
}
