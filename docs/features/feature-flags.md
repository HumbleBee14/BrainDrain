# Feature Flags

**PR:** #19  
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
static config (dev), environment variables, JSON file, or a remote service
like Unleash (production).

## Current flags in use

| Flag | What it controls |
|---|---|
| `billing.outbox.enabled` | Use durable outbox vs in-memory batcher |
| `idempotency.enforced` | Enable API idempotency middleware |

## Provider abstraction

The `FeatureFlagProvider` trait abstracts where flags come from:

| Provider | When to use |
|---|---|
| `StaticProvider` | Development — flags hardcoded in config |
| `EnvProvider` | Staging — flags from environment variables |
| `FileProvider` | Simple production — flags from JSON file |
| `UnleashProvider` | Full production — remote management, targeting |

Switch providers via `FEATURE_FLAGS_PROVIDER` env var.

## Production guard

The platform **refuses to start** in production without
`billing.outbox.enabled=true`. This prevents accidentally running with
the lossy in-memory batcher in production.

## Files

- `crates/api/src/services/feature_flags.rs` — Provider trait + implementations
- `crates/api/src/config.rs` — Provider selection from env
