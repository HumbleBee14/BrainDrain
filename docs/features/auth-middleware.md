# Auth Middleware

**PR:** #21  
**Problem:** Auth was running inside every handler via an Axum extractor.
This meant the idempotency middleware had to duplicate the entire auth chain
(token extraction, Clerk verification, role lookup) just to identify who
made the request. Double auth = double DB calls on every idempotent request.

## How it works

Auth moved from per-handler extractor to a middleware layer that runs once:

```
BEFORE: request → handler → AuthenticatedUser extractor → auth runs here
AFTER:  request → auth_middleware → extensions.insert(user) → handler reads from extensions
```

The `AuthenticatedUser` extractor still works in all 78 handlers — it just
reads from request extensions now instead of running auth itself. **Zero
handler changes.**

## Benefits

- **Single auth execution** — no double DB calls per idempotent request
- **Single auth replacement point** — when swapping Clerk for custom auth,
  change one middleware function
- **Any middleware can read user identity** — idempotency, rate limiting,
  audit logging all read from extensions

## Files

- `crates/api/src/auth.rs` — `auth_middleware()` + `AuthOutcome` extension type
- `crates/api/src/routes/mod.rs` — Middleware applied to v1 router
