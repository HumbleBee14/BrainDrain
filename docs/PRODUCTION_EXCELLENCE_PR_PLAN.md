# Production Excellence PR Plan

This document tracks platform-hardening work for day-0 production excellence.

Last updated: 2026-04-05 (see note below — item 02 was completed the same day, after this table was last edited)

---

## Completed (12 of 12)

| # | Item | PR | What was delivered |
|---|---|---|---|
| 00A | Feature flags foundation | #19 | Provider trait, static provider, 5 flags, production guards |
| 00B | API idempotency foundation | #20 | Middleware, PostgreSQL-backed dedup, body hashing, cleanup |
| 00C | Auth middleware refactor | #21 | Single auth execution path, AuthOutcome in extensions |
| 01 | Durable billing outbox | #22 | Outbox table, relay with savepoints, streaming reservation, deploy billing in same tx |
| 02 | Multi-instance inference control plane | #26 | `inference_instances` table, instance-aware deploy/inference/undeploy, health probes + reconciliation (see below) |
| 03 | PITR backup/restore | #24 | wal-g scripts, WAL archiving, point-in-time restore, backup retention |
| 04 | PgBouncer integration | #24 | Transaction-mode pooler, API+workers routed through, migrations bypass |
| 05 | Release/migration hardening | #24 | Pre-deploy checks, health-gated rollout, migration logging, billing partition verify |
| 06 | Shared Rust/Python codegen | #23 | sync_constants.py generates enums + GPU rates, CI drift check |
| 07 | Feature flag hardening | #25 | Unleash remote provider, graceful fallback, cache on failure, audit logging |
| 08 | Idempotency coverage audit | #23 | Policy documentation, 32-endpoint exhaustive test, zero gaps found |
| 09 | E2E failure-path test matrix | #25 | Billing outbox tests, idempotency exhaustive coverage, feature flag fallback tests |

### Multi-Instance Inference Control Plane (item 02, since completed)

This table originally tracked this as the one remaining item; it shipped in
PR #26 shortly after this table was last written. It solved: the platform
previously had one inference backend — a single `Arc<dyn InferenceBackend>`
created at startup, so every deployment went to the same GPU server, with no
horizontal scaling, capacity tracking, health awareness, or graceful
draining.

**What was delivered:**

- `inference_instances` DB table tracking each GPU server (URL, GPU type,
  adapter capacity, health status, heartbeat)
- Deployments route to a specific instance based on capacity and compatibility
- Adapter count tracked per instance (claim/release pattern)
- Health-based routing: skip unhealthy instances, drain before maintenance
- Reconciliation for stale claims and dead instances
- Deployments are instance-aware, not process-global; inference, undeploy,
  and status operate against the assigned instance; no hard recoupling to
  vLLM was introduced (TGI and SGLang implement the same backend trait)

See [SYSTEM_ARCHITECTURE.md §5](./SYSTEM_ARCHITECTURE.md) for the current
description and its caveat that this control plane, while implemented and
unit-tested, has not yet been proven against sustained production traffic.
