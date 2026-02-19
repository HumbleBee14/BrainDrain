use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

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

    #[error("{message}")]
    Conflict { message: String },

    #[error("Rate limit exceeded")]
    RateLimited,

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
#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
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
            Self::NotImplemented => "NOT_IMPLEMENTED",
            Self::Internal(_) => "INTERNAL_ERROR",
            Self::Database(_) => "DATABASE_ERROR",
            Self::Storage(_) => "STORAGE_ERROR",
        }
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

        let body = ErrorEnvelope {
            error: ErrorBody { code, message },
        };

        (status, axum::Json(body)).into_response()
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
}
