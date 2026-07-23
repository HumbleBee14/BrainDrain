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

This is the only backend component in Python. Reason: the ML libraries it calls (Unsloth, TRL, PyMuPDF, Docling, vLLM) are Python-only. No Rust or Go alternatives exist for these.

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

## Activities

| Activity | Phase | ML Library |
|---|---|---|
| `parse_document` | Phase 1 | PyMuPDF (default) / Docling (optional extra), python-docx |
| `generate_synthetic_pairs` | Phase 1 | Raw HTTP (`httpx`) calls to any OpenAI-compatible endpoint |
| `build_dataset` | Phase 1 | HuggingFace datasets |
| `start_training` | Phase 2 | Unsloth, TRL |
| `run_evaluation` | Phase 3 | Custom + LLM-as-judge |
| `deploy_model` | Phase 4 | vLLM (also supports TGI, SGLang via the control-plane backend abstraction) |

All activities are real, wired implementations with typed dataclass inputs/outputs — not stubs. See [docs/DATA_PIPELINE.md](../../docs/DATA_PIPELINE.md) and [docs/CLOUD_GPU_TRAINING.md](../../docs/CLOUD_GPU_TRAINING.md) for the actual code paths.

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

### Backend Selection

Every processing component uses a **Protocol → Registry → Factory** pattern.
Swap any backend with a single env var — no code changes required.

| Variable | Default | Options | What it controls |
|---|---|---|---|
| `APP_PDF_BACKEND` | `pymupdf` | `pymupdf`, `docling` | PDF extraction library (neither backend does OCR — no path for scanned/image-only PDFs) |
| `APP_LANGUAGE_DETECTOR_BACKEND` | `langdetect` | `langdetect`, `null` | Language detection |
| `APP_TRAINING_ENGINE` | `unsloth` | `unsloth` + custom | Model loading & LoRA |
| `APP_METRICS_BACKEND` | `redis` | `redis`, `log`, `null` | Training metrics sink |
| `APP_EVAL_MODEL_LOADER` | `unsloth` | `unsloth` + custom | Eval model loading |

**Examples:**

```bash
# Use Docling for richer PDF structure (tables, figures, reading order)
APP_PDF_BACKEND=docling  # requires: pip install platform-workers[pdf-ml]

# Local dev without Redis — stream metrics to the logger instead
APP_METRICS_BACKEND=log

# Disable language detection (faster parsing, no langdetect dependency)
APP_LANGUAGE_DETECTOR_BACKEND=null
```

**Registering a custom backend** (no code changes to existing files):

```python
# my_engine.py
from src.activities.training_engine import register_engine

class MyEngine:
    def load_model(self, model_name, max_seq_length, load_in_4bit): ...
    def attach_adapter(self, model, r, lora_alpha, lora_dropout, target_modules): ...
    def save_adapter(self, model, tokenizer, output_dir): ...
    def prepare_for_inference(self, model): return model

register_engine("my_engine", MyEngine)
# Then: APP_TRAINING_ENGINE=my_engine
```

Same pattern works for `register()` in `pdf_backend.py`, `language_detector.py`,
`metrics_collector.py`, and `model_inference.py`.

### Install with ML Dependencies

```bash
uv sync --extra ml        # Unsloth, transformers, datasets, TRL
uv sync --extra pdf-ml    # Docling (alternative PDF backend)
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

## GPU Compute

Training/evaluation dispatch through a `GpuProvider` protocol
(`src/gpu_provider.py`): `LocalGpuProvider` (attached CUDA GPU, the default
and the fully-exercised path) or `ModalGpuProvider` (serverless GPUs on
Modal). The Modal path is validated for deploy and smoke-test runs; a full
train-to-S3 run on cloud infra is not yet proven end-to-end. There is no
RunPod (or other third-party GPU marketplace) integration. See
[docs/CLOUD_GPU_TRAINING.md](../../docs/CLOUD_GPU_TRAINING.md) for the
verified state and how to add a new provider.
