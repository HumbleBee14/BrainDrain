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
| [ARCHITECTURE.md](./ARCHITECTURE.md) | Detailed design rationale, subsystem notes, and technical decisions |
| [DEPLOYMENT.md](./DEPLOYMENT.md) | Serving and deployment architecture, including multi-instance inference |
| [DATA_PIPELINE.md](./DATA_PIPELINE.md) | Ingestion, refinement, and training data flow |

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

| Document | Focus |
|----------|-------|
| [DEVELOPMENT.md](./DEVELOPMENT.md) | Phase-by-phase build log |
| [PRODUCTION_EXCELLENCE_PR_PLAN.md](./PRODUCTION_EXCELLENCE_PR_PLAN.md) | Final production-hardening roadmap and completion record |
| [RESEARCH.md](./RESEARCH.md) | Supporting research and landscape analysis |
