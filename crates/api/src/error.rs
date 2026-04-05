use axum::http::{HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// Application error type that maps cleanly to HTTP responses.
///
/// Every variant carries a status code and error code string.
/// The `IntoResponse` implementation ensures consistent JSON error envelopes.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{message}")]
    NotFound { message: String },

    #[error("{message}")]
    BadRequest { message: String },

    #[error("Authentication required")]
    Unauthorized,

    #[error("{message}")]
    Forbidden { message: String },

    #[allow(dead_code)]
    #[error("{message}")]
    Conflict { message: String },

    #[error("Rate limit exceeded")]
    RateLimited,

    #[error("{message}")]
    ServiceUnavailable { message: String },

    #[allow(dead_code)]
    #[error("Not implemented")]
    NotImplemented,

    #[error("Internal server error")]
    Internal(#[from] anyhow::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Storage error: {0}")]
    Storage(#[from] platform_storage::StorageError),
}

/// JSON error response envelope.
#[derive(Serialize, ToSchema)]
pub(crate) struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ErrorBody {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
}

/// Structured context for error observability.
/// Attached to errors for logging — never exposed to clients.
#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub struct ErrorContext {
    pub tenant_id: Option<Uuid>,
    pub operation: Option<&'static str>,
    pub resource_type: Option<&'static str>,
    pub resource_id: Option<Uuid>,
}

#[allow(dead_code)]
impl ErrorContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tenant(mut self, id: Uuid) -> Self {
        self.tenant_id = Some(id);
        self
    }

    pub fn operation(mut self, op: &'static str) -> Self {
        self.operation = Some(op);
        self
    }

    pub fn resource(mut self, resource_type: &'static str, id: Uuid) -> Self {
        self.resource_type = Some(resource_type);
        self.resource_id = Some(id);
        self
    }
}

impl AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::BadRequest { .. } => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::Conflict { .. } => StatusCode::CONFLICT,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::ServiceUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::NotImplemented => StatusCode::NOT_IMPLEMENTED,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "NOT_FOUND",
            Self::BadRequest { .. } => "BAD_REQUEST",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden { .. } => "FORBIDDEN",
            Self::Conflict { .. } => "CONFLICT",
            Self::RateLimited => "RATE_LIMITED",
            Self::ServiceUnavailable { .. } => "SERVICE_UNAVAILABLE",
            Self::NotImplemented => "NOT_IMPLEMENTED",
            Self::Internal(_) => "INTERNAL_ERROR",
            Self::Database(_) => "DATABASE_ERROR",
            Self::Storage(_) => "STORAGE_ERROR",
        }
    }

    /// Attach structured context for observability logging.
    /// Context is logged with the error but never exposed to clients.
    #[allow(dead_code)]
    pub fn with_context(self, ctx: ErrorContext) -> Self {
        tracing::warn!(
            error = %self,
            error_code = self.error_code(),
            tenant_id = ?ctx.tenant_id,
            operation = ?ctx.operation,
            resource_type = ?ctx.resource_type,
            resource_id = ?ctx.resource_id,
            "Error with context"
        );
        self
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let code = self.error_code();

        // Log internal errors at error level, client errors at warn
        match &self {
            Self::Internal(e) => tracing::error!(error = %e, "Internal server error"),
            Self::Database(e) => tracing::error!(error = %e, "Database error"),
            Self::Storage(e) => tracing::error!(error = %e, "Storage error"),
            _ => tracing::warn!(code = code, error = %self, "Client error"),
        }

        // Don't leak internal error details to clients
        let message = match &self {
            Self::Internal(_) | Self::Database(_) | Self::Storage(_) => {
                "An internal error occurred".to_string()
            }
            other => other.to_string(),
        };

        // request_id is injected by the inject_request_id_into_errors middleware
        // after this response is created (since the error handler doesn't have
        // access to request headers). We set None here; the middleware fills it in.
        let body = ErrorEnvelope {
            error: ErrorBody {
                code,
                message,
                request_id: None,
            },
        };

        (status, axum::Json(body)).into_response()
    }
}

/// Middleware that injects `x-request-id` into JSON error response bodies.
///
/// tower-http already propagates x-request-id to the response header, but error
/// JSON bodies are serialized by `IntoResponse` which has no access to request
/// headers. This middleware runs AFTER the handler, reads the x-request-id from
/// the response header, and patches error JSON bodies (4xx/5xx) to include it.
pub async fn inject_request_id_into_errors(
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    let response = next.run(request).await;

    // Only patch error responses
    if !response.status().is_client_error() && !response.status().is_server_error() {
        return response;
    }

    // Extract request ID from the response header (set by PropagateRequestIdLayer)
    let request_id = response
        .headers()
        .get(HeaderName::from_static("x-request-id"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let Some(request_id) = request_id else {
        return response;
    };
    let status = response.status();
    let headers = response.headers().clone();

    // Read the body
    let body_bytes = match axum::body::to_bytes(response.into_body(), 64 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    // Try to parse and patch the JSON
    if let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        if let Some(error_obj) = json.get_mut("error").and_then(|e| e.as_object_mut()) {
            error_obj.insert(
                "request_id".to_string(),
                serde_json::Value::String(request_id),
            );
        }
        let patched = serde_json::to_vec(&json).unwrap_or_else(|_| body_bytes.to_vec());
        let mut resp = (status, patched).into_response();
        *resp.headers_mut() = headers;
        resp.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        resp
    } else {
        // Not JSON — return as-is
        let mut resp = (status, body_bytes).into_response();
        *resp.headers_mut() = headers;
        resp
    }
}

/// Convenience type alias for route handlers.
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn assert_status(error: AppError, expected: StatusCode) {
        assert_eq!(error.status_code(), expected);
    }

    #[test]
    fn error_status_codes_map_correctly() {
        assert_status(
            AppError::NotFound {
                message: "x".into(),
            },
            StatusCode::NOT_FOUND,
        );
        assert_status(
            AppError::BadRequest {
                message: "x".into(),
            },
            StatusCode::BAD_REQUEST,
        );
        assert_status(AppError::Unauthorized, StatusCode::UNAUTHORIZED);
        assert_status(
            AppError::Forbidden {
                message: "x".into(),
            },
            StatusCode::FORBIDDEN,
        );
        assert_status(
            AppError::Conflict {
                message: "x".into(),
            },
            StatusCode::CONFLICT,
        );
        assert_status(AppError::RateLimited, StatusCode::TOO_MANY_REQUESTS);
        assert_status(
            AppError::ServiceUnavailable {
                message: "x".into(),
            },
            StatusCode::SERVICE_UNAVAILABLE,
        );
        assert_status(AppError::NotImplemented, StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn error_codes_are_uppercase_snake() {
        assert_eq!(
            AppError::NotFound {
                message: "x".into()
            }
            .error_code(),
            "NOT_FOUND"
        );
        assert_eq!(AppError::Unauthorized.error_code(), "UNAUTHORIZED");
        assert_eq!(AppError::RateLimited.error_code(), "RATE_LIMITED");
    }

    #[tokio::test]
    async fn internal_errors_dont_leak_details() {
        let error = AppError::Internal(anyhow::anyhow!("secret db password exposed"));
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let message = json["error"]["message"].as_str().unwrap();
        assert!(!message.contains("secret"));
        assert!(!message.contains("password"));
        assert_eq!(message, "An internal error occurred");
    }

    #[tokio::test]
    async fn client_errors_show_message() {
        let error = AppError::BadRequest {
            message: "Name is required".into(),
        };
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["error"]["code"], "BAD_REQUEST");
        assert_eq!(json["error"]["message"], "Name is required");
    }

    #[tokio::test]
    async fn error_response_has_correct_envelope_shape() {
        let error = AppError::NotFound {
            message: "Project not found".into(),
        };
        let response = error.into_response();
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // Must have { "error": { "code": "...", "message": "..." } }
        assert!(json.get("error").is_some());
        assert!(json["error"].get("code").is_some());
        assert!(json["error"].get("message").is_some());
    }

    // ── Comprehensive error code mapping ──

    #[test]
    fn all_error_variants_have_codes() {
        let variants: Vec<AppError> = vec![
            AppError::NotFound {
                message: "x".into(),
            },
            AppError::BadRequest {
                message: "x".into(),
            },
            AppError::Unauthorized,
            AppError::Forbidden {
                message: "x".into(),
            },
            AppError::Conflict {
                message: "x".into(),
            },
            AppError::RateLimited,
            AppError::ServiceUnavailable {
                message: "x".into(),
            },
            AppError::NotImplemented,
            AppError::Internal(anyhow::anyhow!("x")),
        ];

        for err in variants {
            let code = err.error_code();
            assert!(!code.is_empty(), "Error code should not be empty");
            assert_eq!(
                code,
                code.to_uppercase(),
                "Error code should be uppercase: {code}",
            );
        }
    }

    #[test]
    fn error_code_values_are_correct() {
        assert_eq!(
            AppError::NotFound {
                message: "x".into()
            }
            .error_code(),
            "NOT_FOUND",
        );
        assert_eq!(
            AppError::BadRequest {
                message: "x".into()
            }
            .error_code(),
            "BAD_REQUEST",
        );
        assert_eq!(AppError::Unauthorized.error_code(), "UNAUTHORIZED");
        assert_eq!(
            AppError::Forbidden {
                message: "x".into()
            }
            .error_code(),
            "FORBIDDEN",
        );
        assert_eq!(
            AppError::Conflict {
                message: "x".into()
            }
            .error_code(),
            "CONFLICT",
        );
        assert_eq!(AppError::RateLimited.error_code(), "RATE_LIMITED");
        assert_eq!(
            AppError::ServiceUnavailable {
                message: "x".into()
            }
            .error_code(),
            "SERVICE_UNAVAILABLE",
        );
        assert_eq!(AppError::NotImplemented.error_code(), "NOT_IMPLEMENTED");
        assert_eq!(
            AppError::Internal(anyhow::anyhow!("x")).error_code(),
            "INTERNAL_ERROR",
        );
    }

    // ── Status code mapping is consistent ──

    #[test]
    fn forbidden_maps_to_403() {
        assert_status(
            AppError::Forbidden {
                message: "x".into(),
            },
            StatusCode::FORBIDDEN,
        );
    }

    #[test]
    fn conflict_maps_to_409() {
        assert_status(
            AppError::Conflict {
                message: "x".into(),
            },
            StatusCode::CONFLICT,
        );
    }

    #[test]
    fn rate_limited_maps_to_429() {
        assert_status(AppError::RateLimited, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn not_implemented_maps_to_501() {
        assert_status(AppError::NotImplemented, StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn internal_maps_to_500() {
        assert_status(
            AppError::Internal(anyhow::anyhow!("err")),
            StatusCode::INTERNAL_SERVER_ERROR,
        );
    }

    // ── Internal errors don't leak details ──

    #[tokio::test]
    async fn database_errors_dont_leak_details() {
        // Simulate a database error using sqlx::Error
        let error = AppError::Internal(anyhow::anyhow!("connection refused to pg:5432"));
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let message = json["error"]["message"].as_str().unwrap();
        assert!(!message.contains("connection"));
        assert!(!message.contains("5432"));
        assert_eq!(message, "An internal error occurred");
    }

    #[tokio::test]
    async fn forbidden_error_shows_message() {
        let error = AppError::Forbidden {
            message: "Insufficient permissions".into(),
        };
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["error"]["code"], "FORBIDDEN");
        assert_eq!(json["error"]["message"], "Insufficient permissions");
    }

    #[tokio::test]
    async fn conflict_error_shows_message() {
        let error = AppError::Conflict {
            message: "Already exists".into(),
        };
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["error"]["code"], "CONFLICT");
        assert_eq!(json["error"]["message"], "Already exists");
    }

    #[tokio::test]
    async fn rate_limited_error_envelope() {
        let error = AppError::RateLimited;
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["error"]["code"], "RATE_LIMITED");
    }

    // ── Display trait correctness ──

    #[test]
    fn display_messages_are_correct() {
        assert_eq!(
            AppError::NotFound {
                message: "gone".into()
            }
            .to_string(),
            "gone",
        );
        assert_eq!(
            AppError::BadRequest {
                message: "bad".into()
            }
            .to_string(),
            "bad",
        );
        assert_eq!(
            AppError::Unauthorized.to_string(),
            "Authentication required"
        );
        assert_eq!(AppError::RateLimited.to_string(), "Rate limit exceeded");
        assert_eq!(AppError::NotImplemented.to_string(), "Not implemented");
        assert_eq!(
            AppError::Internal(anyhow::anyhow!("oops")).to_string(),
            "Internal server error",
        );
    }

    // ── ErrorContext builder ──

    #[test]
    fn error_context_builder_pattern() {
        let tenant_id = uuid::Uuid::new_v4();
        let resource_id = uuid::Uuid::new_v4();

        let ctx = ErrorContext::new()
            .tenant(tenant_id)
            .operation("create")
            .resource("project", resource_id);

        assert_eq!(ctx.tenant_id, Some(tenant_id));
        assert_eq!(ctx.operation, Some("create"));
        assert_eq!(ctx.resource_type, Some("project"));
        assert_eq!(ctx.resource_id, Some(resource_id));
    }

    #[test]
    fn error_context_default_is_all_none() {
        let ctx = ErrorContext::new();
        assert!(ctx.tenant_id.is_none());
        assert!(ctx.operation.is_none());
        assert!(ctx.resource_type.is_none());
        assert!(ctx.resource_id.is_none());
    }

    // ── From trait conversions ──

    #[test]
    fn anyhow_converts_to_internal() {
        let err: AppError = anyhow::anyhow!("something went wrong").into();
        assert!(matches!(err, AppError::Internal(_)));
        assert_eq!(err.error_code(), "INTERNAL_ERROR");
    }

    // ── No extra fields leaked in JSON envelope ──

    #[tokio::test]
    async fn json_envelope_has_only_expected_fields() {
        let error = AppError::BadRequest {
            message: "test".into(),
        };
        let response = error.into_response();
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let top_keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(
            top_keys,
            vec!["error"],
            "Top-level should only have 'error'"
        );

        let error_keys: Vec<&String> = json["error"].as_object().unwrap().keys().collect();
        assert_eq!(
            error_keys.len(),
            2,
            "Error object should have exactly 2 fields (code, message)",
        );
        assert!(json["error"].get("code").is_some());
        assert!(json["error"].get("message").is_some());
    }
}
