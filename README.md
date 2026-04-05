# BrainDrain

An end-to-end platform for turning raw documents into deployed, fine-tuned LLMs. Upload files, the platform handles everything else: parsing, synthetic data generation, training (SFT/DPO/GRPO), evaluation, and deployment with OpenAI-compatible inference.

## Architecture

```
Internet
   |
   +-- app.domain.com --> Next.js (dashboard)
   |
   +-- api.domain.com --> Rust/Axum API --> PgBouncer --> PostgreSQL
                              |                              |
                              +-- Redis (cache, rate limit)  |
                              +-- S3/MinIO (storage)         |
                              +-- Temporal (workflows) ------+-- Python Workers
                              +-- GPU Servers (vLLM/TGI/SGLang)
```

Three core services, independently deployable:

| Service | Stack | What it does |
|---|---|---|
| **API** | Rust (Axum, SQLx) | Auth, CRUD, billing, inference routing, deployment control plane |
| **Workers** | Python (Temporal, Unsloth, TRL) | Document parsing, data generation, training, evaluation |
| **Web** | Next.js 15 | Dashboard UI with real-time status |

## Quick Start

```bash
make infra          # PostgreSQL, Redis, MinIO
make temporal       # Temporal server
make migrate        # Run DB migrations
make dev-api        # Rust API on :8000
make dev-web        # Next.js on :3000
make dev-workers    # Python workers
```

## Documentation

| Document | What it covers |
|---|---|
| [Project Flow](docs/PROJECT_FLOW.md) | End-to-end pipeline: upload to deployed model |
| [Architecture](docs/SYSTEM_ARCHITECTURE.md) | System design, control plane, infrastructure |
| [Research](docs/RESEARCH.md) | LLM training/serving landscape analysis |
