# apps/workers

**Python Temporal workers — executes the ML pipeline (parsing, data generation, training, evaluation, deployment).**

| | |
|---|---|
| Language | Python 3.11 |
| Orchestration | Temporal SDK |
| Package manager | uv |
| Deploys as | Docker container on GPU machines |
| Task queue | `ml-pipeline` |
| Depends on | Temporal server, PostgreSQL, S3/MinIO |

## Why Python (Not Rust)

This is the only backend component in Python. Reason: the ML libraries it calls (Unsloth, TRL, distilabel, MinerU, vLLM) are Python-only. No Rust or Go alternatives exist for these.

## Pipeline Stages

```
Upload → Parse → Refine → Train → Evaluate → Deploy
```

Each stage is a **Temporal activity** (individually retryable, observable in Temporal UI).

## Workflows

| Workflow | What It Does | Timeout |
|---|---|---|
| `IngestWorkflow` | Parse uploaded documents (PDF, DOCX, etc.) | 10 min/doc |
| `RefineWorkflow` | Generate synthetic instruction/response pairs | 30 min |
| `TrainWorkflow` | Fine-tune model via Unsloth/TRL | 6 hours |
| `EvaluateWorkflow` | Score model quality (LLM-as-judge + metrics) | 1 hour |
| `FullPipelineWorkflow` | Chains all stages end-to-end | Sum of above |

## Activities (Currently Stubs)

| Activity | Phase | ML Library |
|---|---|---|
| `parse_document` | Phase 1 | MinerU, python-docx |
| `generate_synthetic_pairs` | Phase 1 | distilabel |
| `build_dataset` | Phase 1 | HuggingFace datasets |
| `start_training` | Phase 2 | Unsloth, TRL |
| `run_evaluation` | Phase 3 | Custom + LLM-as-judge |
| `deploy_model` | Phase 4 | vLLM |

All activities have **typed dataclass inputs and outputs** — ready to fill in with real ML code.

## Running Locally

```bash
# 1. Prerequisites: Temporal server running
docker compose -f infra/temporal/docker-compose.temporal.yml up -d

# 2. Install dependencies
uv sync

# 3. Start worker
uv run python -m src.worker
```

### Required Environment Variables

| Variable | Default | Description |
|---|---|---|
| `APP_TEMPORAL_ADDRESS` | `localhost:7233` | Temporal gRPC endpoint |
| `APP_TEMPORAL_NAMESPACE` | `default` | Temporal namespace |
| `APP_TEMPORAL_TASK_QUEUE` | `ml-pipeline` | Task queue name |
| `APP_DATABASE_URL` | `postgresql://...@localhost:5432/platform` | PostgreSQL connection |
| `APP_S3_ENDPOINT` | `http://localhost:9000` | MinIO/S3 endpoint |
| `APP_S3_ACCESS_KEY` | `minioadmin` | S3 access key |
| `APP_S3_SECRET_KEY` | `minioadmin` | S3 secret key |

All prefixed with `APP_` (configurable via pydantic-settings).

### Install with ML Dependencies

```bash
uv sync --extra ml    # Includes unsloth, transformers, distilabel, datasets
```

## Docker Build & Deploy

```bash
# Build image
docker build -f apps/workers/Dockerfile -t platform-workers apps/workers/

# Run container (CPU — for parsing/data gen)
docker run --env-file .env platform-workers

# Run container (GPU — for training)
docker run --gpus all --env-file .env platform-workers
```

Final image is **~500MB+** (Python + ML dependencies).
