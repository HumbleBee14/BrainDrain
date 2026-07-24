use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{Method, Request, Response};
use axum::middleware::Next;
use jsonwebtoken::{DecodingKey, TokenData, Validation, decode};
use platform_shared::enums::TeamRole;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::error::AppError;

// ---------------------------------------------------------------------------
// JWKS cache — avoids fetching keys on every request
// ---------------------------------------------------------------------------

/// Cached JWKS key set with TTL-based expiry.
///
/// Keys are stored by `kid` for O(1) lookup. The cache refreshes
/// when the TTL expires or when a JWT presents a `kid` not in the cache
/// (key rotation scenario).
#[derive(Clone)]
pub struct JwksCache {
    inner: Arc<RwLock<JwksCacheInner>>,
    jwks_url: String,
    http_client: reqwest::Client,
}

struct JwksCacheInner {
    /// kid → (n, e) RSA components
    keys: HashMap<String, (String, String)>,
    /// When the cache was last refreshed
    fetched_at: Option<Instant>,
}

const JWKS_CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour
const JWKS_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

impl JwksCache {
    pub fn new(jwks_url: String, http_client: reqwest::Client) -> Self {
        Self {
            inner: Arc::new(RwLock::new(JwksCacheInner {
                keys: HashMap::new(),
                fetched_at: None,
            })),
            jwks_url,
            http_client,
        }
    }

    /// Get the decoding key for a given `kid`. Refreshes cache if needed.
    async fn get_key(&self, kid: &str) -> Result<DecodingKey, AppError> {
        // Fast path: cache hit within TTL
        {
            let cache = self.inner.read().await;
            if let Some(fetched_at) = cache.fetched_at
                && fetched_at.elapsed() < JWKS_CACHE_TTL
                && let Some((n, e)) = cache.keys.get(kid)
            {
                return DecodingKey::from_rsa_components(n, e).map_err(|_| AppError::Unauthorized);
            }
        }

        // Slow path: refresh cache
        self.refresh().await?;

        let cache = self.inner.read().await;
        let (n, e) = cache.keys.get(kid).ok_or(AppError::Unauthorized)?;
        DecodingKey::from_rsa_components(n, e).map_err(|_| AppError::Unauthorized)
    }

    /// Fetch JWKS from the provider and update the cache.
    async fn refresh(&self) -> Result<(), AppError> {
        if self.jwks_url.is_empty() {
            return Err(AppError::Unauthorized);
        }

        let jwks: serde_json::Value = self
            .http_client
            .get(&self.jwks_url)
            .timeout(JWKS_FETCH_TIMEOUT)
            .send()
            .await
            .map_err(|_| AppError::Unauthorized)?
            .json()
            .await
            .map_err(|_| AppError::Unauthorized)?;

        let keys_array = jwks["keys"].as_array().ok_or(AppError::Unauthorized)?;

        let mut key_map = HashMap::new();
        for key in keys_array {
            if let (Some(kid), Some(n), Some(e)) =
                (key["kid"].as_str(), key["n"].as_str(), key["e"].as_str())
            {
                key_map.insert(kid.to_string(), (n.to_string(), e.to_string()));
            }
        }

        if key_map.is_empty() {
            return Err(AppError::Unauthorized);
        }

        let mut cache = self.inner.write().await;
        cache.keys = key_map;
        cache.fetched_at = Some(Instant::now());

        Ok(())
    }
}

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
    /// Email address from the auth token, if the provider supplies it. Used for
    /// the platform-admin email allowlist. `None` for dev/internal tokens.
    pub email: Option<String>,
    /// Team role for RBAC enforcement.
    pub role: TeamRole,
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
/// corresponding tenant from the database. JWKS keys are cached
/// with a 1-hour TTL and refreshed on cache miss (key rotation).
pub struct ClerkAuthProvider {
    jwks_cache: JwksCache,
    is_dev: bool,
    /// Expected `iss` claim. `None` disables issuer validation.
    issuer: Option<String>,
    /// Allowlist for the `azp` claim. Empty disables the check.
    authorized_parties: Vec<String>,
}

impl ClerkAuthProvider {
    pub fn new(
        jwks_url: String,
        is_dev: bool,
        http_client: reqwest::Client,
        issuer: Option<String>,
        authorized_parties: Vec<String>,
    ) -> Self {
        Self {
            jwks_cache: JwksCache::new(jwks_url, http_client),
            is_dev,
            issuer,
            authorized_parties,
        }
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

            // Try Clerk JWT verification (uses cached JWKS keys)
            let claims = match verify_clerk_jwt(
                token,
                &self.jwks_cache,
                self.issuer.as_deref(),
                &self.authorized_parties,
            )
            .await
            {
                Ok(c) => c,
                Err(e) => return Some(Err(e)),
            };

            let tenant_id = if let Some(ref org_id) = claims.org_id {
                match resolve_tenant_id(db, org_id).await {
                    Ok(id) => id,
                    Err(e) => return Some(Err(e)),
                }
            } else {
                match resolve_personal_tenant(db, &claims.sub, claims.email.as_deref()).await {
                    Ok(id) => id,
                    Err(e) => return Some(Err(e)),
                }
            };

            Some(Ok(AuthenticatedUser {
                user_id: claims.sub,
                tenant_id,
                org_id: claims.org_id,
                email: claims.email,
                role: TeamRole::Member,
            }))
        })
    }
}

// ---------------------------------------------------------------------------
// InternalTokenAuthProvider — worker → API service calls
// ---------------------------------------------------------------------------

/// Authenticates internal service-to-service calls using a shared secret.
///
/// The worker sends `Authorization: Bearer {platform_internal_token}` plus
/// an `X-Tenant-Id` header. This provider validates the token and extracts
/// the tenant UUID from the header. The authenticated user gets `Owner` role
/// but is restricted to specific API paths (deploy callbacks only).
///
/// **Security:** Internal auth is scoped to `INTERNAL_AUTH_ALLOWED_SUFFIXES` to
/// prevent a compromised worker from accessing arbitrary tenant endpoints.
pub struct InternalTokenAuthProvider {
    token: String,
}

/// Exact path suffixes where internal token auth is allowed.
/// Matched directly against the end of the full request path using `ends_with`.
/// All other routes reject internal tokens and fall through to JWT auth.
const INTERNAL_AUTH_ALLOWED_SUFFIXES: &[&str] = &[
    "/deploy",   // POST /api/v1/models/{id}/deploy
    "/undeploy", // POST /api/v1/models/{id}/undeploy
];

impl InternalTokenAuthProvider {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    /// Check if bearer token matches the internal token for an allowed path.
    /// Returns `None` if this isn't an internal token (let other providers try).
    /// Uses constant-time comparison (via `subtle`) to prevent timing side-channel attacks.
    pub fn authenticate_with_headers(
        &self,
        token: &str,
        method: &Method,
        headers: &axum::http::HeaderMap,
        request_path: &str,
    ) -> Option<Result<AuthenticatedUser, AppError>> {
        if self.token.is_empty() {
            return None;
        }

        // Constant-time comparison: hash both tokens with SHA-256, then compare
        // digests using subtle::ConstantTimeEq. GenericArray's PartialEq uses
        // standard short-circuit comparison — subtle guarantees fixed-time.
        use sha2::{Digest, Sha256};
        use subtle::ConstantTimeEq;
        let expected = Sha256::digest(self.token.as_bytes());
        let provided = Sha256::digest(token.as_bytes());
        if expected.ct_eq(&provided).unwrap_u8() != 1 {
            return None;
        }

        // Scope check: only allow internal auth on POST deploy/undeploy routes
        let path_allowed = *method == Method::POST
            && INTERNAL_AUTH_ALLOWED_SUFFIXES
                .iter()
                .any(|suffix| request_path.ends_with(suffix));
        if !path_allowed {
            tracing::warn!(
                path = request_path,
                "Internal token used on non-allowed path — rejecting"
            );
            return None;
        }

        let tenant_id = match headers
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<Uuid>().ok())
        {
            Some(id) => id,
            None => {
                return Some(Err(AppError::BadRequest {
                    message: "Internal auth requires X-Tenant-Id header".to_string(),
                }));
            }
        };

        Some(Ok(AuthenticatedUser {
            user_id: "__internal_worker__".to_string(),
            tenant_id,
            org_id: None,
            email: None,
            role: TeamRole::Owner,
        }))
    }
}

// ---------------------------------------------------------------------------
// FromRequestParts extractor
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Auth helpers — reusable primitives for all auth paths
// ---------------------------------------------------------------------------

/// Extract the Bearer token from an Authorization header value.
pub fn extract_bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
}

/// Authenticate a bearer token and return the identity.
///
/// Tries internal token auth first (path-scoped), then falls through to the
/// auth provider chain. Returns the raw identity — call
/// `resolve_role_and_bootstrap` to get the full app principal with team role.
pub async fn authenticate_token(
    state: &AppState,
    token: &str,
    method: &Method,
    headers: &axum::http::HeaderMap,
    path: &str,
) -> Result<AuthenticatedUser, AppError> {
    // Internal token auth (worker → API calls, POST deploy/undeploy only)
    if let Some(internal_provider) = state.internal_auth()
        && let Some(result) =
            internal_provider.authenticate_with_headers(token, method, headers, path)
    {
        return result;
    }

    state.auth_chain().authenticate(token, state.db()).await
}

/// A cloneable auth error stored in extensions when authentication or
/// authorization fails. Preserves the original HTTP status code and message
/// so the extractor can return 403/400 instead of collapsing to 401.
#[derive(Debug, Clone)]
pub struct AuthError {
    pub status: axum::http::StatusCode,
    pub message: String,
}

impl From<&AppError> for AuthError {
    fn from(e: &AppError) -> Self {
        use axum::http::StatusCode;
        let (status, message) = match e {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Authentication required".into()),
            AppError::Forbidden { message } => (StatusCode::FORBIDDEN, message.clone()),
            AppError::BadRequest { message } => (StatusCode::BAD_REQUEST, message.clone()),
            // Infrastructure failures (DB down, storage error, etc.) → 500.
            // Never collapse these to 401 — they are server errors, not auth errors.
            AppError::Database(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Authentication service unavailable: {e}"),
            ),
            AppError::Internal(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Authentication service error: {e}"),
            ),
            other => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Authentication service error: {other}"),
            ),
        };
        Self { status, message }
    }
}

impl From<AuthError> for AppError {
    fn from(e: AuthError) -> Self {
        use axum::http::StatusCode;
        match e.status {
            StatusCode::FORBIDDEN => AppError::Forbidden { message: e.message },
            StatusCode::BAD_REQUEST => AppError::BadRequest { message: e.message },
            StatusCode::INTERNAL_SERVER_ERROR => {
                AppError::Internal(anyhow::anyhow!("{}", e.message))
            }
            _ => AppError::Unauthorized,
        }
    }
}

/// The auth outcome stored in request extensions by the middleware.
///
/// Wraps the full `Result` so the extractor can return the original error
/// (e.g., 403 Forbidden for non-members) instead of collapsing to 401.
#[derive(Clone)]
pub struct AuthOutcome(pub Result<AuthenticatedUser, AuthError>);

/// Canonical accessor for the authenticated principal from request extensions.
///
/// Returns `Some(&user)` if auth succeeded, `None` if either:
/// - No Bearer header was present (unauthenticated request), or
/// - Auth was attempted but failed (invalid token, forbidden, DB error).
///
/// To distinguish these cases, read `AuthOutcome` from extensions directly.
#[allow(dead_code)] // Public API for future middleware (rate limiting, audit, feature flags)
pub fn request_principal(parts: &Parts) -> Option<&AuthenticatedUser> {
    parts
        .extensions
        .get::<AuthOutcome>()
        .and_then(|outcome| outcome.0.as_ref().ok())
}

// ---------------------------------------------------------------------------
// Auth middleware — runs once per request, stores result in extensions
// ---------------------------------------------------------------------------

/// Axum middleware that authenticates the request and stores the
/// `AuthenticatedUser` in request extensions.
///
/// Does NOT reject unauthenticated requests — that's the extractor's job.
/// This allows routes that optionally use auth (WebSocket via query param)
/// to coexist under the same router without exclusion lists.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response<Body> {
    if let Some(token) = extract_bearer_token(request.headers()) {
        let token = token.to_string();
        let outcome = match authenticate_token(
            &state,
            &token,
            request.method(),
            request.headers(),
            request.uri().path(),
        )
        .await
        {
            Ok(mut user) => match resolve_role_and_bootstrap(&state, &mut user).await {
                Ok(()) => AuthOutcome(Ok(user)),
                Err(e) => AuthOutcome(Err(AuthError::from(&e))),
            },
            Err(e) => AuthOutcome(Err(AuthError::from(&e))),
        };
        request.extensions_mut().insert(outcome);
    }
    // No Bearer header → no AuthOutcome in extensions → extractor returns 401.

    next.run(request).await
}

/// Seed default notification preferences for a freshly bootstrapped owner.
/// In-app is always on; email is added when the JWT carries an address.
/// Best-effort — a failure here never blocks sign-in.
async fn seed_default_notification_prefs(state: &AppState, user: &AuthenticatedUser) {
    let email_config = user
        .email
        .as_deref()
        .filter(|e| !e.is_empty())
        .map(|e| serde_json::json!({ "address": e }));

    for event_type in platform_shared::events::notification::DEFAULTS {
        if let Err(e) = state
            .notification_repo()
            .upsert_preference(
                user.tenant_id,
                "in_app",
                event_type,
                true,
                serde_json::json!({}),
            )
            .await
        {
            tracing::warn!(error = %e, event_type, "Failed to seed default in-app preference");
        }

        if let Some(ref config) = email_config
            && let Err(e) = state
                .notification_repo()
                .upsert_preference(user.tenant_id, "email", event_type, true, config.clone())
                .await
        {
            tracing::warn!(error = %e, event_type, "Failed to seed default email preference");
        }
    }
}

/// Resolve the user's role from `team_members` and auto-bootstrap the first
/// user as Owner if the tenant has no members yet.
///
/// Public so the WebSocket handler (which authenticates via query param) can
/// reuse the same logic.
pub async fn resolve_role_and_bootstrap(
    state: &AppState,
    user: &mut AuthenticatedUser,
) -> Result<(), AppError> {
    // Dev tokens keep their assigned role (Owner) — skip role lookup.
    if user.role == TeamRole::Owner && user.org_id.is_none() {
        return Ok(());
    }

    match state
        .team_member_repo()
        .get_role(user.tenant_id, &user.user_id)
        .await?
    {
        Some(role_str) => {
            user.role = match role_str.parse() {
                Ok(r) => r,
                Err(_) => {
                    tracing::error!(
                        tenant_id = %user.tenant_id,
                        user_id = %user.user_id,
                        role = %role_str,
                        "Corrupted role in team_members — defaulting to Member"
                    );
                    TeamRole::Member
                }
            };
        }
        None => {
            // No team_member row — auto-bootstrap if tenant has zero members.
            let count = state
                .team_member_repo()
                .count_by_tenant(user.tenant_id)
                .await?;
            if count == 0 {
                match state
                    .team_member_repo()
                    .create(
                        user.tenant_id,
                        &user.user_id,
                        "", // email not available from JWT
                        "owner",
                        None,
                    )
                    .await
                {
                    Ok(member) => {
                        if member.user_id == user.user_id {
                            user.role = member.role.parse().unwrap_or(TeamRole::Owner);
                            seed_default_notification_prefs(state, user).await;
                        } else {
                            return Err(AppError::Forbidden {
                                message: "You are not a member of this team. Ask an admin for an invitation.".to_string(),
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, tenant_id = %user.tenant_id, "Owner auto-bootstrap failed");
                        return Err(AppError::Forbidden {
                            message:
                                "You are not a member of this team. Ask an admin for an invitation."
                                    .to_string(),
                        });
                    }
                }
            } else {
                return Err(AppError::Forbidden {
                    message: "You are not a member of this team. Ask an admin for an invitation."
                        .to_string(),
                });
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Handler extractor — reads from extensions (zero-cost after middleware)
// ---------------------------------------------------------------------------

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<AuthOutcome>() {
            Some(AuthOutcome(Ok(user))) => Ok(user.clone()),
            Some(AuthOutcome(Err(e))) => Err(AppError::from(e.clone())),
            None => Err(AppError::Unauthorized),
        }
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
    /// Present only when the Clerk JWT template includes the email claim.
    #[serde(default)]
    email: Option<String>,
    /// Authorized party — the frontend origin that minted the session.
    #[serde(default)]
    azp: Option<String>,
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
            email: None,
            role: TeamRole::Owner,
        })
    } else {
        None
    }
}

/// Verify a Clerk JWT using cached JWKS keys.
///
/// Extracts the `kid` from the JWT header to find the correct signing key.
/// Uses the JWKS cache (1h TTL) to avoid per-request HTTP calls.
/// Validates `iss` when `issuer` is set and `azp` when `authorized_parties`
/// is non-empty (both opt-in — unset config keeps prior behavior).
async fn verify_clerk_jwt(
    token: &str,
    jwks_cache: &JwksCache,
    issuer: Option<&str>,
    authorized_parties: &[String],
) -> Result<ClerkClaims, AppError> {
    // Extract kid from JWT header for key matching
    let header = jsonwebtoken::decode_header(token).map_err(|_| AppError::Unauthorized)?;
    let kid = header.kid.ok_or(AppError::Unauthorized)?;

    // Get the decoding key from cache (auto-refreshes on miss or TTL expiry)
    let decoding_key = jwks_cache.get_key(&kid).await?;

    let validation = build_validation(issuer);

    let TokenData { claims, .. } = decode::<ClerkClaims>(token, &decoding_key, &validation)
        .map_err(|_| AppError::Unauthorized)?;

    check_azp(claims.azp.as_deref(), authorized_parties)?;

    Ok(claims)
}

/// Build JWT validation rules: RS256, expiry, and (when configured) issuer.
fn build_validation(issuer: Option<&str>) -> Validation {
    let mut validation = Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.validate_exp = true;
    if let Some(iss) = issuer {
        validation.set_issuer(&[iss]);
    }
    validation
}

/// Enforce the `azp` allowlist. Empty allowlist ⇒ check disabled.
/// A missing `azp` claim is rejected when the allowlist is configured.
fn check_azp(azp: Option<&str>, authorized_parties: &[String]) -> Result<(), AppError> {
    if authorized_parties.is_empty() {
        return Ok(());
    }
    match azp {
        Some(azp) if authorized_parties.iter().any(|p| p == azp) => Ok(()),
        _ => Err(AppError::Unauthorized),
    }
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

/// For users without an org, resolve their personal tenant, creating it on
/// first sign-in. Personal tenants use the user ID as the clerk_org_id; the
/// no-op ON CONFLICT update makes concurrent first requests race-safe.
async fn resolve_personal_tenant(
    db: &sqlx::PgPool,
    clerk_user_id: &str,
    email: Option<&str>,
) -> Result<Uuid, AppError> {
    let name = email
        .filter(|e| !e.is_empty())
        .unwrap_or("Personal Workspace");

    sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO tenants (clerk_org_id, name)
        VALUES ($1, $2)
        ON CONFLICT (clerk_org_id) DO UPDATE SET clerk_org_id = EXCLUDED.clerk_org_id
        RETURNING id
        "#,
    )
    .bind(clerk_user_id)
    .bind(name)
    .fetch_one(db)
    .await
    .map_err(AppError::Database)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};

    // Throwaway RSA keypair generated for these tests only — never used anywhere else.
    const TEST_RSA_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQCp965pbEZ3b8/5
R8hIKWTB+hoIXrnHOa+zThSlzdeh2Ugcl+3pv5oEiGGB7uqoEtodYrMnBxONBzUm
CjqjtAvstFtnLEArlW/fQ5OZzKeMj+r/6BrDNjZh4QNHnieOfkHGIy4o76Hh5nBN
jjrMl7GjabA+5IzdoKNG2lLqqLZHhDORNAXF5NRnayYX/36fS9lluL+Hw7OTpskr
Qh9pbZAJinS6ehcM6rsBDWvjLwnw+YZpfi9VEV72xENFaB09kIzs3KOlcNCzqkkN
llRaklfRE7suR+7AblBi6gpNZVkPjTrwa1+TlaLFYdgNxMjgrvmjzWaWeKyIW7St
qTVwKXyFAgMBAAECggEAaoQU4m5/jrQcwt0wb8C5KzNAg0RR6r+FE7p4CByC6SQR
JBI2gAmaTQLnEJWYqzH9TPMg0PGHWBdPQIKikxrvaizxJyw9HtMs498mrfjqe5Vp
sWxU8UeVNyvbcVN0+MC5GaHMeM0MR1Sxxni+8p6SLZW7ZP64JOBZ0rpZwkNu0EvM
yEPe43MkN1/rxbgIGABYVId6sna0IlgYLfiwHRgykaWQIygdFPZQiXmbmfgi6e7a
kk2N6OsMPu9Rkv68vGngmh5SRwrl6hshrZImOaeZ+TWJuyxS2SwKVcGllJQtF6de
FBX8S8L7SeXaYuFjkG+JXGvMk+XJOAh760aRsdRadQKBgQDTYiv14T5Z3+hU9I0a
fjPIf9KxG0BBNwKT/smejiSh8izzlLJfdSN0JAPluGR108ajwX1IqeC7izUEI+xB
fFLDbmjff6KBfgtQnFVpIyUcXZG0o6HE13uQWMW0rqORbuwiJP4ey2eaoISY6eHf
BGv1MmpddnA/4ZTSA1hDcxYxiwKBgQDN16bWTBhEH6EQt+oa+jDA1z1UtwpNcvkm
NSXlOsyRtIHgQqTkK+5DBNdHTXruwWe9Loz1KfpY0ooZ4tIIZtOH++gOCfYlPe3u
ztBJrjS03GIM0fSbdJVBsIW8tUdYSvCbqYMUOrFYKleeluUWCrELot1BI0JncRSc
0Y6KgQesLwKBgEByZO7BLq5eGsqUCNUz9vvBJO6EXXHEoM+YVcY2liqd2GCnTD7Y
Sufk9x85ub9GwwA4RMc7q93iElbh0O0iR2V4KxdBJb2PPUnlcBDu+yiLypmlbfPC
stSOjDCLMilsBShf2O5wm3TETckFPa0t/vAx38YBDzYaw7HH/UgLNZADAoGAdGJt
W5dU1RfJGsnSHQS/Ehng/Igt1BKg2sCMN6riRbP5BxLHZpeMNOqEyjT9wAcsn6O1
YV0lxpjsKqy7srJpAeclkuKBARed8zuOO0q7VFOTQMppcogdaDHlvAgHWd2tY2YZ
zhNNeJsgRXPt/WN4LSsdzJmiDxi53d0Cqj9AVlMCgYAvSKaT4IWS0SPXFOn3GB5q
tqablmqHMf61wSUogkYiNS/uJxZ2AcaV6s1f7kqjFEj0ICShvXbmQImZO44ETwK4
y3bvuKI4OIXzTbh6jhhQzx9BusRMH5OWZDpQye7Q0Y7+jFUCmc94LPHiopyLGf7N
e/WXHQfJavZ36z2IOfjqoQ==
-----END PRIVATE KEY-----";

    const TEST_RSA_PUBLIC_PEM: &str = "-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAqfeuaWxGd2/P+UfISClk
wfoaCF65xzmvs04Upc3XodlIHJft6b+aBIhhge7qqBLaHWKzJwcTjQc1Jgo6o7QL
7LRbZyxAK5Vv30OTmcynjI/q/+gawzY2YeEDR54njn5BxiMuKO+h4eZwTY46zJex
o2mwPuSM3aCjRtpS6qi2R4QzkTQFxeTUZ2smF/9+n0vZZbi/h8Ozk6bJK0IfaW2Q
CYp0unoXDOq7AQ1r4y8J8PmGaX4vVRFe9sRDRWgdPZCM7NyjpXDQs6pJDZZUWpJX
0RO7LkfuwG5QYuoKTWVZD4068Gtfk5WixWHYDcTI4K75o81mlnisiFu0rak1cCl8
hQIDAQAB
-----END PUBLIC KEY-----";

    fn sign_token(iss: &str) -> String {
        let exp = chrono::Utc::now().timestamp() + 300;
        let claims = serde_json::json!({ "sub": "user_1", "iss": iss, "exp": exp });
        let key = EncodingKey::from_rsa_pem(TEST_RSA_PRIVATE_PEM.as_bytes()).unwrap();
        encode(&Header::new(jsonwebtoken::Algorithm::RS256), &claims, &key).unwrap()
    }

    fn decode_with_issuer(token: &str, issuer: Option<&str>) -> Result<ClerkClaims, ()> {
        let key = DecodingKey::from_rsa_pem(TEST_RSA_PUBLIC_PEM.as_bytes()).unwrap();
        decode::<ClerkClaims>(token, &key, &build_validation(issuer))
            .map(|d| d.claims)
            .map_err(|_| ())
    }

    #[test]
    fn issuer_match_accepted() {
        let token = sign_token("https://good.example.com");
        let claims = decode_with_issuer(&token, Some("https://good.example.com")).unwrap();
        assert_eq!(claims.sub, "user_1");
    }

    #[test]
    fn issuer_mismatch_rejected() {
        let token = sign_token("https://evil.example.com");
        assert!(decode_with_issuer(&token, Some("https://good.example.com")).is_err());
    }

    #[test]
    fn issuer_unset_skips_check() {
        let token = sign_token("https://anything.example.com");
        assert!(decode_with_issuer(&token, None).is_ok());
    }

    #[test]
    fn azp_empty_allowlist_allows_any() {
        assert!(check_azp(Some("https://app.example.com"), &[]).is_ok());
        assert!(check_azp(None, &[]).is_ok());
    }

    #[test]
    fn azp_in_allowlist_accepted() {
        let allowed = vec!["https://app.example.com".to_string()];
        assert!(check_azp(Some("https://app.example.com"), &allowed).is_ok());
    }

    #[test]
    fn azp_not_in_allowlist_rejected() {
        let allowed = vec!["https://app.example.com".to_string()];
        assert!(check_azp(Some("https://evil.example.com"), &allowed).is_err());
    }

    #[test]
    fn azp_missing_rejected_when_allowlist_set() {
        let allowed = vec!["https://app.example.com".to_string()];
        assert!(check_azp(None, &allowed).is_err());
    }
}
