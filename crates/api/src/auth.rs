use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{Request, Response};
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
}

impl ClerkAuthProvider {
    pub fn new(jwks_url: String, is_dev: bool, http_client: reqwest::Client) -> Self {
        Self {
            jwks_cache: JwksCache::new(jwks_url, http_client),
            is_dev,
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
            let claims = match verify_clerk_jwt(token, &self.jwks_cache).await {
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
/// **Security:** Internal auth is scoped to `INTERNAL_AUTH_ALLOWED_PATHS` to
/// prevent a compromised worker from accessing arbitrary tenant endpoints.
pub struct InternalTokenAuthProvider {
    token: String,
}

/// Path prefixes where internal token auth is allowed.
/// All other routes reject internal tokens and fall through to Clerk JWT auth.
const INTERNAL_AUTH_ALLOWED_PATHS: &[&str] = &[
    "/api/v1/models/", // deploy/undeploy callbacks
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

        // Scope check: only allow internal auth on specific route prefixes
        let path_allowed = INTERNAL_AUTH_ALLOWED_PATHS
            .iter()
            .any(|prefix| request_path.starts_with(prefix));
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
            role: TeamRole::Owner,
        }))
    }
}

// ---------------------------------------------------------------------------
// FromRequestParts extractor
// ---------------------------------------------------------------------------

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
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    if let Some(token) = token {
        // Try internal token first (worker → API calls, path-scoped)
        let auth_result = if let Some(internal_provider) = state.internal_auth() {
            if let Some(result) = internal_provider.authenticate_with_headers(
                token,
                request.headers(),
                request.uri().path(),
            ) {
                Some(result)
            } else {
                Some(state.auth_chain().authenticate(token, state.db()).await)
            }
        } else {
            Some(state.auth_chain().authenticate(token, state.db()).await)
        };

        if let Some(Ok(mut user)) = auth_result {
            // Resolve role from team_members + auto-bootstrap first user
            match resolve_role_and_bootstrap(&state, &mut user).await {
                Ok(()) => {
                    request.extensions_mut().insert(user);
                }
                Err(e) => {
                    // Role resolution failed (forbidden, DB error) — don't store user.
                    // The handler extractor will return 401/403 when it can't find the user.
                    tracing::debug!(error = %e, "Auth middleware: role resolution failed");
                }
            }
        }
        // Auth failed — don't reject, let handler extractor return 401.
    }

    next.run(request).await
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
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or(AppError::Unauthorized)
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
async fn verify_clerk_jwt(token: &str, jwks_cache: &JwksCache) -> Result<ClerkClaims, AppError> {
    // Extract kid from JWT header for key matching
    let header = jsonwebtoken::decode_header(token).map_err(|_| AppError::Unauthorized)?;
    let kid = header.kid.ok_or(AppError::Unauthorized)?;

    // Get the decoding key from cache (auto-refreshes on miss or TTL expiry)
    let decoding_key = jwks_cache.get_key(&kid).await?;

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
