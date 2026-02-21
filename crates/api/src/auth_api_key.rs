use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::error::AppError;
use crate::services::api_key_service::ApiKeyService;

/// Authenticated API key extracted from `Authorization: Bearer pl_sk_...`.
///
/// Separate from `AuthenticatedUser` (Clerk JWT). This extractor is used
/// for inference endpoints where external developers call the model API
/// with a platform API key.
#[derive(Debug, Clone)]
pub struct ApiKeyAuth {
    pub key_id: Uuid,
    pub tenant_id: Uuid,
    pub model_id: Uuid,
}

impl FromRequestParts<AppState> for ApiKeyAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(AppError::Unauthorized)?;

        let raw_key = auth_header
            .strip_prefix("Bearer ")
            .ok_or(AppError::Unauthorized)?;

        // API keys start with pl_sk_
        if !raw_key.starts_with(platform_shared::constants::API_KEY_PREFIX) {
            return Err(AppError::Unauthorized);
        }

        let mut redis = state.redis();

        let authenticated =
            ApiKeyService::authenticate(state.api_key_repo(), &mut redis, raw_key).await?;

        Ok(ApiKeyAuth {
            key_id: authenticated.key_id,
            tenant_id: authenticated.tenant_id,
            model_id: authenticated.model_id,
        })
    }
}
