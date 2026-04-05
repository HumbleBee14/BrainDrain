# Production Excellence PR Plan

This document tracks platform-hardening work for day-0 production excellence.

Last updated: 2026-04-05

---

## Completed (11 of 12)

| # | Item | PR | What was delivered |
|---|---|---|---|
| 00A | Feature flags foundation | #19 | Provider trait, static provider, 5 flags, production guards |
| 00B | API idempotency foundation | #20 | Middleware, PostgreSQL-backed dedup, body hashing, cleanup |
| 00C | Auth middleware refactor | #21 | Single auth execution path, AuthOutcome in extensions |
| 01 | Durable billing outbox | #22 | Outbox table, relay with savepoints, streaming reservation, deploy billing in same tx |
| 02 | — | — | **PENDING: Multi-instance inference control plane** |
| 03 | PITR backup/restore | #24 | wal-g scripts, WAL archiving, point-in-time restore, backup retention |
| 04 | PgBouncer integration | #24 | Transaction-mode pooler, API+workers routed through, migrations bypass |
| 05 | Release/migration hardening | #24 | Pre-deploy checks, health-gated rollout, migration logging, billing partition verify |
| 06 | Shared Rust/Python codegen | #23 | sync_constants.py generates enums + GPU rates, CI drift check |
| 07 | Feature flag hardening | #25 | Unleash remote provider, graceful fallback, cache on failure, audit logging |
| 08 | Idempotency coverage audit | #23 | Policy documentation, 32-endpoint exhaustive test, zero gaps found |
| 09 | E2E failure-path test matrix | #25 | Billing outbox tests, idempotency exhaustive coverage, feature flag fallback tests |

## Remaining (1 of 12)

### Multi-Instance Inference Control Plane

This is the only remaining architectural gap. Everything else is complete.

**What it solves:** Today the platform has one inference backend — a single
`Arc<dyn InferenceBackend>` created at startup. Every deployment goes to the
same GPU server. This prevents horizontal scaling, capacity tracking, health
awareness, and graceful draining.

**Target design:**

- `inference_instances` DB table tracking each GPU server (URL, GPU type,
  adapter capacity, health status, heartbeat)
- Route deployments to specific instances based on capacity and compatibility
- Track adapter count per instance (claim/release pattern)
- Health-based routing: skip unhealthy instances, drain before maintenance
- Reconciliation for stale claims and dead instances

**Acceptance criteria:**

- Deployments are instance-aware, not process-global
- Inference, undeploy, and status operate against the assigned instance
- Capacity is explicit, auditable, and repairable
- No hard recoupling to vLLM is introduced

**When to do this:** After the platform actually needs 2+ GPU servers.
This is a scaling feature, not a correctness feature. Everything else
(billing, idempotency, auth, backup, pooling, flags) was correctness
and operational safety — those came first intentionally.
