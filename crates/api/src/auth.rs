use std::future::Future;
use std::pin::Pin;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use jsonwebtoken::{DecodingKey, TokenData, Validation, decode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::error::AppError;

/// Boxed future returned by [`AuthProvider::authenticate`].
type AuthFuture<'a> =
    Pin<Box<dyn Future<Output = Option<Result<AuthenticatedUser, AppError>>> + Send + 'a>>;

/// Authenticated user extracted from a JWT or other auth token.
///
/// Available as an extractor in route handlers:
/// ```ignore
/// async fn handler(user: AuthenticatedUser) -> impl IntoResponse { ... }
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct AuthenticatedUser {
    /// User ID (e.g. Clerk sub claim, OAuth2 subject).
    pub user_id: String,
    /// Tenant UUID (mapped from external org/user identity).
    pub tenant_id: Uuid,
    /// External organization ID (raw), if any.
    pub org_id: Option<String>,
}

// ---------------------------------------------------------------------------
// AuthProvider trait
// ---------------------------------------------------------------------------

/// Trait for authentication providers.
/// Implement this to add new auth methods (OAuth2, SAML, OpenID Connect).
pub trait AuthProvider: Send + Sync + 'static {
    /// Attempt to authenticate from the given bearer token.
    /// Returns `None` if this provider doesn't handle the given credentials.
    /// Returns `Some(Ok(user))` on success, `Some(Err(e))` on auth failure.
    fn authenticate<'a>(&'a self, token: &'a str, db: &'a sqlx::PgPool) -> AuthFuture<'a>;
}

// ---------------------------------------------------------------------------
// AuthProviderChain
// ---------------------------------------------------------------------------

/// Chain of auth providers. Tries each in order until one handles the request.
pub struct AuthProviderChain {
    providers: Vec<Box<dyn AuthProvider>>,
}

impl AuthProviderChain {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn add(mut self, provider: impl AuthProvider) -> Self {
        self.providers.push(Box::new(provider));
        self
    }

    pub async fn authenticate(
        &self,
        token: &str,
        db: &sqlx::PgPool,
    ) -> Result<AuthenticatedUser, AppError> {
        for provider in &self.providers {
            if let Some(result) = provider.authenticate(token, db).await {
                return result;
            }
        }
        Err(AppError::Unauthorized)
    }
}

// ---------------------------------------------------------------------------
// ClerkAuthProvider
// ---------------------------------------------------------------------------

/// Clerk JWT authentication provider.
///
/// Verifies tokens against Clerk's JWKS endpoint and resolves the
/// corresponding tenant from the database.
pub struct ClerkAuthProvider {
    jwks_url: String,
    is_dev: bool,
}

impl ClerkAuthProvider {
    pub fn new(jwks_url: String, is_dev: bool) -> Self {
        Self { jwks_url, is_dev }
    }
}

impl AuthProvider for ClerkAuthProvider {
    fn authenticate<'a>(&'a self, token: &'a str, db: &'a sqlx::PgPool) -> AuthFuture<'a> {
        Box::pin(async move {
            // Dev token check (only in development mode)
            if self.is_dev
                && let Some(dev_user) = parse_dev_token(token)
            {
                return Some(Ok(dev_user));
            }

            // Try Clerk JWT verification
            let claims = match verify_clerk_jwt(token, &self.jwks_url).await {
                Ok(c) => c,
                Err(e) => return Some(Err(e)),
            };

            let tenant_id = if let Some(ref org_id) = claims.org_id {
                match resolve_tenant_id(db, org_id).await {
                    Ok(id) => id,
                    Err(e) => return Some(Err(e)),
                }
            } else {
                match resolve_personal_tenant(db, &claims.sub).await {
                    Ok(id) => id,
                    Err(e) => return Some(Err(e)),
                }
            };

            Some(Ok(AuthenticatedUser {
                user_id: claims.sub,
                tenant_id,
                org_id: claims.org_id,
            }))
        })
    }
}

// ---------------------------------------------------------------------------
// FromRequestParts extractor
// ---------------------------------------------------------------------------

impl FromRequestParts<AppState> for AuthenticatedUser {
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

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(AppError::Unauthorized)?;

        state.auth_chain().authenticate(token, state.db()).await
    }
}

// ---------------------------------------------------------------------------
// Internal helpers (used by ClerkAuthProvider)
// ---------------------------------------------------------------------------

/// JWT claims from Clerk.
#[derive(Debug, Deserialize)]
struct ClerkClaims {
    sub: String,
    org_id: Option<String>,
    // Clerk includes more claims; we only extract what we need.
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
