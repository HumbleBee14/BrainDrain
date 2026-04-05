# Feature Flags

**PRs:** #19 (foundation), #25 (hardening)  
**Problem:** You want to deploy new code but only enable it for some tenants,
or be able to instantly disable a feature if it causes problems — without
redeploying.

## How it works

Instead of `if (config.new_feature)`, code checks a feature flag:

```rust
if state.feature_flags().is_enabled("billing.outbox.enabled", &context) {
    // durable path
} else {
    // legacy path
}
```

The flag value can come from different sources depending on your needs:
static JSON config (dev/staging) or a remote Unleash server (production).

## Current flags in use

| Flag | What it controls |
|---|---|
| `billing.outbox.enabled` | Use durable outbox vs in-memory batcher |
| `idempotency.enforced` | Enable API idempotency middleware |
| `deployments.multi_instance.enabled` | Multi-instance deployment routing |
| `notifications.delivery_worker.enabled` | Background notification delivery |
| `inference.backend.tgi.enabled` | TGI inference backend |

## Providers

| Provider | `FEATURE_FLAGS_PROVIDER` | When to use |
|---|---|---|
| **Static** | `static` (default) | Dev/staging — flags from JSON file or env var |
| **Unleash** | `unleash` | Production — remote management with kill switches |

### Static provider

Set flags via `FEATURE_FLAGS_JSON` or `FEATURE_FLAGS_FILE`:

```bash
FEATURE_FLAGS_JSON='{"billing.outbox.enabled":true,"idempotency.enforced":true}'
```

### Unleash provider (self-hosted, free)

Unleash OSS is a self-hosted feature flag server. We poll its API every 15
seconds. No paid service required.

```bash
FEATURE_FLAGS_PROVIDER=unleash
UNLEASH_URL=http://unleash:4242
UNLEASH_API_TOKEN=your-client-token
UNLEASH_APP_NAME=platform-api
UNLEASH_ENVIRONMENT=production
```

**Failure behavior:**
- **Startup, Unleash unreachable:** Falls back to static config (`FEATURE_FLAGS_JSON`).
  Does not crash. Logs a warning.
- **Runtime, Unleash goes down:** Keeps serving from the last successfully
  fetched cache. Logs warnings on each failed poll.
- **Runtime, Unleash comes back:** Resumes polling, updates cache.

**Audit trail:** Every flag change between poll cycles is logged at `WARN`
level with old and new values. Unleash server also has a native audit log UI.

**Kill switch pattern:** Toggle any flag in the Unleash UI → takes effect
within 15 seconds on all API instances.

## Production guards

The platform **refuses to start** in production without
`billing.outbox.enabled=true` and rejects TGI backend without its flag
enabled. These are startup-only checks — they prevent dangerous
misconfigurations, not runtime flag changes.

## Files

- `crates/api/src/services/feature_flags.rs` — Trait, providers, poller
- `crates/api/src/config.rs` — Provider selection from env
- `crates/api/src/app_state.rs` — Initialization + shutdown wiring
