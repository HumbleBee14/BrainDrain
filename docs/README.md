# Docs Index

This folder contains the current architecture, operations, and product-flow documentation for BrainDrain.

## Start Here

| Document | Why read it |
|----------|-------------|
| [PROJECT_FLOW.md](./PROJECT_FLOW.md) | End-to-end product flow from upload to deployed model |
| [QUICKSTART.md](./QUICKSTART.md) | Local setup and first-run commands |
| [SYSTEM_ARCHITECTURE.md](./SYSTEM_ARCHITECTURE.md) | Current production system map |

## Architecture

| Document | Focus |
|----------|-------|
| [ARCHITECTURE.md](./ARCHITECTURE.md) | **Original design doc (Feb 2026), aspirational** — design rationale and technical decisions at project inception; see its banner for how it diverges from what was built |
| [DEPLOYMENT.md](./DEPLOYMENT.md) | Serving and deployment architecture, including multi-instance inference |
| [DATA_PIPELINE.md](./DATA_PIPELINE.md) | Ingestion, refinement, and training data flow |
| [CLOUD_GPU_TRAINING.md](./CLOUD_GPU_TRAINING.md) | Cloud GPU training (Modal): verified state, reservation pattern, known crash-window caveats |

## Production Operations

| Document | Focus |
|----------|-------|
| [PRODUCTION_OPS.md](./PRODUCTION_OPS.md) | PITR, PgBouncer, release checks, and runbooks |
| [PRODUCTION_SETUP_GUIDE.md](./PRODUCTION_SETUP_GUIDE.md) | End-to-end production setup guide |
| [PRODUCTION_SCALE.md](./PRODUCTION_SCALE.md) | Scale posture and future operational expansion notes |

## Feature and Subsystem Docs

See the feature docs under [features/](./features/) for focused notes on:
- auth middleware
- API idempotency
- durable billing outbox
- feature flags
- release pipeline
- PITR backup
- PgBouncer

## Build History and Research

These are dated, historical records (development trackers and phase
completion reports) — read them as a log of what happened at the time, not
as a live description of the current system. `phase0/` through `phase5/`
completion reports follow the same pattern.

| Document | Focus |
|----------|-------|
| [DEVELOPMENT.md](./DEVELOPMENT.md) | Phase-by-phase build log |
| [PRODUCTION_EXCELLENCE_PR_PLAN.md](./PRODUCTION_EXCELLENCE_PR_PLAN.md) | Final production-hardening roadmap and completion record |
| [RESEARCH.md](./RESEARCH.md) | **Landscape research (Feb 2026), aspirational** — market/tooling survey from project inception, not all of it adopted; see its banner |
