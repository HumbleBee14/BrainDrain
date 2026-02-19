use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use jsonwebtoken::{DecodingKey, TokenData, Validation, decode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::error::AppError;

/// Authenticated user extracted from a Clerk JWT.
///
/// Available as an extractor in route handlers:
/// ```ignore
/// async fn handler(user: AuthenticatedUser) -> impl IntoResponse { ... }
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct AuthenticatedUser {
    /// Clerk user ID (sub claim).
    pub user_id: String,
    /// Tenant UUID (from org_id claim, mapped to our tenants table).
    pub tenant_id: Uuid,
    /// Clerk organization ID (raw).
    pub org_id: Option<String>,
}

/// JWT claims from Clerk.
#[derive(Debug, Deserialize)]
struct ClerkClaims {
    sub: String,
    org_id: Option<String>,
    // Clerk includes more claims; we only extract what we need.
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Extract Bearer token from Authorization header
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(AppError::Unauthorized)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(AppError::Unauthorized)?;

        // In development mode, accept a simple dev token format: "dev_{tenant_id}_{user_id}"
        if state.config().is_dev()
            && let Some(dev_user) = parse_dev_token(token)
        {
            return Ok(dev_user);
        }

        // Verify JWT with Clerk's JWKS
        let claims = verify_clerk_jwt(token, &state.config().clerk_jwks_url).await?;

        // Map org_id to tenant_id by looking up the tenant in the database
        let tenant_id = if let Some(ref org_id) = claims.org_id {
            resolve_tenant_id(state.db(), org_id).await?
        } else {
            // Personal workspace — resolve by user's personal org
            resolve_personal_tenant(state.db(), &claims.sub).await?
        };

        Ok(AuthenticatedUser {
            user_id: claims.sub,
            tenant_id,
            org_id: claims.org_id,
        })
    }
}

/// Parse a development-only token for local testing.
/// Format: `dev_{tenant_uuid}_{user_id}`
fn parse_dev_token(token: &str) -> Option<AuthenticatedUser> {
    let parts: Vec<&str> = token.splitn(3, '_').collect();
    if parts.len() == 3 && parts[0] == "dev" {
        let tenant_id = Uuid::parse_str(parts[1]).ok()?;
        Some(AuthenticatedUser {
            user_id: parts[2].to_string(),
            tenant_id,
            org_id: None,
        })
    } else {
        None
    }
}

/// Verify a Clerk JWT using the JWKS endpoint.
async fn verify_clerk_jwt(token: &str, jwks_url: &str) -> Result<ClerkClaims, AppError> {
    if jwks_url.is_empty() {
        return Err(AppError::Unauthorized);
    }

    // Fetch JWKS (in production, this should be cached with TTL)
    let jwks: serde_json::Value = reqwest::get(jwks_url)
        .await
        .map_err(|_| AppError::Unauthorized)?
        .json()
        .await
        .map_err(|_| AppError::Unauthorized)?;

    // Get the first RSA key from JWKS
    let key = jwks["keys"]
        .as_array()
        .and_then(|keys| keys.first())
        .ok_or(AppError::Unauthorized)?;

    let n = key["n"].as_str().ok_or(AppError::Unauthorized)?;
    let e = key["e"].as_str().ok_or(AppError::Unauthorized)?;

    let decoding_key =
        DecodingKey::from_rsa_components(n, e).map_err(|_| AppError::Unauthorized)?;

    let mut validation = Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.validate_exp = true;

    let TokenData { claims, .. } = decode::<ClerkClaims>(token, &decoding_key, &validation)
        .map_err(|_| AppError::Unauthorized)?;

    Ok(claims)
}

/// Look up tenant_id by Clerk organization ID.
async fn resolve_tenant_id(db: &sqlx::PgPool, clerk_org_id: &str) -> Result<Uuid, AppError> {
    let row = sqlx::query_scalar::<_, Uuid>("SELECT id FROM tenants WHERE clerk_org_id = $1")
        .bind(clerk_org_id)
        .fetch_optional(db)
        .await
        .map_err(AppError::Database)?;

    row.ok_or_else(|| AppError::Forbidden {
        message: "Organization not found. Please set up your workspace first.".to_string(),
    })
}

/// For users without an org, resolve their personal tenant.
async fn resolve_personal_tenant(db: &sqlx::PgPool, clerk_user_id: &str) -> Result<Uuid, AppError> {
    // Personal tenants use the user ID as the clerk_org_id
    let row = sqlx::query_scalar::<_, Uuid>("SELECT id FROM tenants WHERE clerk_org_id = $1")
        .bind(clerk_user_id)
        .fetch_optional(db)
        .await
        .map_err(AppError::Database)?;

    row.ok_or_else(|| AppError::Forbidden {
        message: "Tenant not found. Please complete onboarding.".to_string(),
    })
}
