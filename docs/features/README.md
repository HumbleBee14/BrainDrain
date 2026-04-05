# Platform Features Guide

This folder documents every production-scale feature added to the platform,
explaining **what** it does, **why** it's needed, and **how** it works at a
high level. Start here if you're new or returning after a break.

---

## Feature Index

| Feature | Doc | PR | What it solves |
|---|---|---|---|
| Inference backend abstraction | [inference-backends.md](inference-backends.md) | #18 | Support multiple GPU serving engines without code changes |
| Feature flags | [feature-flags.md](feature-flags.md) | #19 | Toggle features safely without redeploying |
| API idempotency | [api-idempotency.md](api-idempotency.md) | #20 | Prevent duplicate side effects from client retries |
| Auth middleware | [auth-middleware.md](auth-middleware.md) | #21 | Single auth execution path, no double-auth |
| Durable billing outbox | [billing-outbox.md](billing-outbox.md) | #22 | Never lose billing events, even if the server crashes |
| Rust/Python codegen sync | [codegen-sync.md](codegen-sync.md) | #23 | Prevent Rust and Python constants from drifting apart |
| PgBouncer connection pooling | [pgbouncer.md](pgbouncer.md) | #24 | Handle hundreds of app connections with fewer Postgres connections |
| PITR backup and restore | [pitr-backup.md](pitr-backup.md) | #24 | Recover the database to any point in time after a disaster |
| Release pipeline hardening | [release-pipeline.md](release-pipeline.md) | #24 | Safe, ordered deployments with health checks |
| Multi-instance inference control plane | [multi-instance-inference.md](multi-instance-inference.md) | #26 | Route deploy / inference / undeploy through registered serving instances |
| Pre-commit hooks | [pre-commit-hooks.md](pre-commit-hooks.md) | #23 | Auto-format code before every commit, catch drift |
