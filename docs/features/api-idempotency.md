# API Idempotency

**PR:** #20  
**Problem:** If a client sends a "create project" request and the network drops
before they get the response, they'll retry. Without idempotency, you get two
projects. This applies to every mutating operation — billing, deployments,
team invitations.

## How it works

The client sends an `Idempotency-Key` header with a unique value (usually a
UUID). The server stores the request+response keyed by this value. If the
same key comes in again:

- **Same request body:** Return the cached response (no duplicate side effect)
- **Different request body:** Return 400 (key reuse with different data)
- **Still processing:** Return 409 (concurrent duplicate)

```
Client: POST /api/v1/projects  Idempotency-Key: abc-123  → 200 {project}
Client: POST /api/v1/projects  Idempotency-Key: abc-123  → 200 {same project, from cache}
```

## Coverage

35 mutating endpoints are covered, with 3 intentional exclusions:

| Covered | Why |
|---|---|
| All `/api/v1/*` POST/PUT/DELETE | Business operations that must not duplicate |

| Excluded | Why |
|---|---|
| `/v1/chat/completions` | Inference — API key auth, non-idempotent by design |
| `/api/webhooks/stripe` | HMAC auth, no user context, Stripe retries natively |
| Document uploads | Multipart bodies too large to buffer for dedup |

## How it's implemented

- **Middleware layer** — runs before every handler, no per-route code needed
- **PostgreSQL-backed** — idempotency keys stored in `idempotency_keys` table
- **Scoped per user+tenant+method+route** — no cross-user replay
- **1 MB body limit** — larger requests skip idempotency transparently
- **Auto-cleanup** — expired and stale keys cleaned up periodically
- **Feature flag** — `IDEMPOTENCY_ENFORCED` controls global enablement

## Files

- `crates/api/src/services/idempotency.rs` — Middleware + tests
- `crates/db/src/migrations/011_idempotency_keys.sql` — Schema
