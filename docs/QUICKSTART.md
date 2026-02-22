# BrainDrain — Quickstart Guide

> How to set up, run, test, and deploy every component of the platform.

---

## Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Docker + Docker Compose | 24+ | [docker.com](https://docker.com) |
| Rust | 1.75+ | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Node.js | 20+ | [nodejs.org](https://nodejs.org) |
| pnpm | 9+ | `npm install -g pnpm` |
| Python | 3.11+ | [python.org](https://python.org) |
| uv | Latest | `pip install uv` |

---

## 1. Start Infrastructure

```bash
# Using Makefile targets (recommended):
make infra           # Start PostgreSQL 16 + Redis 7 + MinIO (also creates docker network)
make temporal        # Start Temporal server (separate — heavyweight)

# Verify everything is healthy:
docker compose ps
docker compose -f infra/temporal/docker-compose.temporal.yml ps
```

Or manually:
```bash
docker network create platform-net       # One-time
docker compose up -d                     # PostgreSQL, Redis, MinIO
docker compose -f infra/temporal/docker-compose.temporal.yml up -d
```

**What's now running:**

| Service | URL | Credentials |
|---------|-----|-------------|
| PostgreSQL | `localhost:5432` | user: `platform` / pass: `platform_dev` / db: `platform` |
| Redis | `localhost:6379` | No auth (max 256mb, allkeys-lru) |
| MinIO Console | `http://localhost:9001` | `minioadmin` / `minioadmin` |
| MinIO S3 API | `http://localhost:9000` | `minioadmin` / `minioadmin` |
| Temporal Server | `localhost:7233` | — |
| Temporal UI | `http://localhost:8088` | — |

---

## 2. Configure Environment

### Rust API (.env at project root)

```bash
# Copy from the template:
cp .env.example .env

# Edit if needed — defaults work for local development with Docker Compose.
# Key vars (all have sensible defaults in .env.example):
#   DATABASE_URL, REDIS_URL, S3_ENDPOINT, S3_ACCESS_KEY, S3_SECRET_KEY, S3_BUCKET
#   TEMPORAL_HOST, VLLM_API_URL, CLERK_JWKS_URL, CORS_ORIGINS
#   STRIPE_SECRET_KEY (optional — leave empty for NoOp billing)
```

### Next.js Frontend (apps/web/.env.local)

```bash
cp apps/web/.env.local.example apps/web/.env.local

# Edit if needed. Key vars:
#   NEXT_PUBLIC_API_URL=http://localhost:8000
#   NEXT_PUBLIC_APP_NAME=BrainDrain
#   NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY (leave empty for dev mode)
```

### Python Workers (apps/workers/.env)

Workers read env vars with `APP_` prefix. Create `apps/workers/.env`:

```bash
cat > apps/workers/.env << 'EOF'
# Temporal
APP_TEMPORAL_ADDRESS=localhost:7233
APP_TEMPORAL_NAMESPACE=default
APP_TEMPORAL_TASK_QUEUE=ml-pipeline

# Database (workers read directly for some operations)
APP_DATABASE_URL=postgresql://platform:platform_dev@localhost:5432/platform

# Redis
APP_REDIS_URL=redis://localhost:6379

# S3 (MinIO)
APP_S3_ENDPOINT=http://localhost:9000
APP_S3_ACCESS_KEY=minioadmin
APP_S3_SECRET_KEY=minioadmin
APP_S3_BUCKET=platform-dev

# Platform API (workers call back to the API)
APP_PLATFORM_API_URL=http://localhost:8000

# LLM API — platform-wide defaults (per-tenant settings via API take priority — see Section 8)
APP_LLM_API_BASE_URL=https://api.openai.com/v1
APP_LLM_API_KEY=
APP_LLM_MODEL=gpt-4o-mini

# GPU/ML (needed for training — Section 9)
APP_HF_TOKEN=
APP_WORKER_MODE=all

# vLLM (needed for deployment — Section 10)
APP_VLLM_API_URL=http://localhost:8080
EOF
```

---

## 3. Run Database Migrations

```bash
make migrate
# Runs: cargo run -p platform-db --bin migrate
# Creates all tables, indexes, RLS policies, billing partitions (8 migrations)
```

---

## 4. Start the Rust API

```bash
# Terminal 1
make dev-api
# Runs: cargo run -p platform-api
# API available at http://localhost:8000
# Swagger UI at http://localhost:8000/docs (dev mode only)
```

**Quick health check:**
```bash
curl http://localhost:8000/health
# → {"status":"ok","version":"..."}

curl http://localhost:8000/ready
# → {"status":"ok","postgres":true,"redis":true}
```

**Explore the API:** Open `http://localhost:8000/docs` for interactive Swagger UI with all 53+ endpoints, request/response schemas, and an "Authorize" button for JWT/API key auth.

---

## 5. Start the Frontend

```bash
# Terminal 2
cd apps/web
pnpm install    # first time only
pnpm dev
# Frontend available at http://localhost:3000
```

**Note:** Without Clerk keys configured, the frontend auth flow won't work. Use the API directly with dev tokens (Section 7) for initial testing.

---

## 6. Start the Python Workers

```bash
# Terminal 3
cd apps/workers
uv sync         # first time only — installs Python dependencies
uv run python -m src.worker
# Workers connect to Temporal and listen on task queues
```

**Worker modes** (set via `APP_WORKER_MODE` in `.env`):
- `all` — Dev mode: registers all activities on `ml-pipeline` queue (default)
- `main` — CPU only: listens on `ml-pipeline-main`, registers parsing/chunking/synthesis
- `gpu` — GPU only: listens on `ml-pipeline-gpu`, registers training/evaluation

---

## 7. Test the Basic Flow (No GPU Required)

### Via API (using dev tokens)

The API supports dev tokens for local testing without Clerk. Format: `dev_{tenant_uuid}_{user_id}`

```bash
# Generate a test token:
TENANT_ID=$(python3 -c "import uuid; print(uuid.uuid4())")
TOKEN="dev_${TENANT_ID}_testuser"
API=http://localhost:8000/api/v1

# 1. Create a project
curl -s -X POST $API/projects \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "My First Project", "description": "Testing the pipeline", "task_type": "qa"}' | jq .

# Save the project ID from the response:
PROJECT_ID=<id-from-response>

# 2. Upload a document
curl -s -X POST "$API/projects/$PROJECT_ID/documents" \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@path/to/your/document.pdf" | jq .

# 3. Trigger parsing (requires Temporal + Python worker running)
curl -s -X POST "$API/projects/$PROJECT_ID/parse" \
  -H "Authorization: Bearer $TOKEN" | jq .

# 4. Check pipeline status (poll until parsing completes)
curl -s "$API/projects/$PROJECT_ID/status" \
  -H "Authorization: Bearer $TOKEN" | jq .

# 5. List parsed documents
curl -s "$API/projects/$PROJECT_ID/documents" \
  -H "Authorization: Bearer $TOKEN" | jq .
```

### Via Swagger UI

1. Open `http://localhost:8000/docs`
2. Click "Authorize" → paste your dev token (`dev_{uuid}_testuser`)
3. Try endpoints interactively

---

## 8. Test Data Pipeline (Requires LLM API Key)

Synthetic data generation, DPO/GRPO training judges, and evaluation all need an LLM API key (any OpenAI-compatible provider works). There are two ways to configure it:

### Option A: Per-Tenant Settings (via API — recommended)

Configure LLM provider through the settings API. This is stored in the database per-tenant and takes priority over env vars.

```bash
# Set your LLM provider (admin role required)
curl -s -X PUT "$API/settings/llm" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "provider": "openai",
    "api_base_url": "https://api.openai.com/v1",
    "api_key": "sk-proj-your-key-here",
    "model": "gpt-4o-mini",
    "max_tokens": 2000
  }' | jq .
# → Returns settings with api_key masked: "sk-p...here"

# Verify your config:
curl -s "$API/settings/llm" \
  -H "Authorization: Bearer $TOKEN" | jq .

# To reset back to platform defaults:
curl -s -X DELETE "$API/settings/llm" \
  -H "Authorization: Bearer $TOKEN"
```

Works with any OpenAI-compatible provider:

| Provider | api_base_url | Example model |
|----------|-------------|---------------|
| OpenAI | `https://api.openai.com/v1` | `gpt-4o-mini` |
| Groq | `https://api.groq.com/openai/v1` | `llama-3.1-70b-versatile` |
| Together AI | `https://api.together.xyz/v1` | `meta-llama/Llama-3.1-8B-Instruct-Turbo` |
| Ollama (local) | `http://localhost:11434/v1` | `llama3.1` |
| Any OpenAI-compatible | Your URL | Your model |

### Option B: Worker Environment Variables (platform-wide default)

Set in `apps/workers/.env` — applies to all tenants that haven't configured their own provider:

```bash
APP_LLM_API_BASE_URL=https://api.openai.com/v1
APP_LLM_API_KEY=sk-your-openai-key
APP_LLM_MODEL=gpt-4o-mini
```

> **How it works:** Workers always check the tenant's DB settings first. If the tenant has no custom config, they fall back to these env vars. If neither is set, the pipeline fails with a clear error message.

### Trigger the pipeline

```bash
# Trigger refine (chunks docs → generates Q&A pairs → builds dataset)
curl -s -X POST "$API/projects/$PROJECT_ID/refine" \
  -H "Authorization: Bearer $TOKEN" | jq .

# Monitor in Temporal UI: http://localhost:8088

# After completion, list datasets:
curl -s "$API/projects/$PROJECT_ID/datasets" \
  -H "Authorization: Bearer $TOKEN" | jq .

# Preview generated training pairs:
DATASET_ID=<id-from-response>
curl -s "$API/datasets/$DATASET_ID/preview" \
  -H "Authorization: Bearer $TOKEN" | jq .
```

---

## 9. Training (Requires NVIDIA GPU)

Training requires a machine with an NVIDIA GPU (A10G+ recommended for 7B models).

### Setup

```bash
# In apps/workers/.env:
APP_HF_TOKEN=hf_your_huggingface_token   # For downloading base models
APP_WORKER_MODE=all                       # or "gpu" for GPU-only worker
```

Install ML dependencies (optional group):
```bash
cd apps/workers
uv sync --extra ml    # Installs unsloth, transformers, trl, etc.
```

### GPU Provider (local vs. serverless)

Training dispatches through a pluggable GPU provider. Set `APP_GPU_PROVIDER` in `apps/workers/.env`:

| Provider | Config | When to use |
|----------|--------|-------------|
| **Local** (default) | `APP_GPU_PROVIDER=local` | Dev/testing on your own GPU. No extra setup. |
| **Modal** (serverless) | `APP_GPU_PROVIDER=modal` | Production. Auto-provisions cloud GPUs, scales to zero. |

**Local (default — no config needed):**
```bash
# Just start the worker. It uses whatever GPU is on the machine.
APP_GPU_PROVIDER=local    # This is the default, you can omit it
```

**Modal (serverless GPUs for production):**
```bash
# 1. Install Modal
pip install modal    # or: uv sync --extra gpu-cloud

# 2. Add to apps/workers/.env:
APP_GPU_PROVIDER=modal
MODAL_TOKEN_ID=ak-xxxxx        # From https://modal.com/settings
MODAL_TOKEN_SECRET=as-xxxxx

# 3. That's it — training jobs auto-provision GPUs on Modal
```

### Create a training job

```bash
curl -s -X POST "$API/projects/$PROJECT_ID/training-jobs" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "dataset_id": "'$DATASET_ID'",
    "base_model": "unsloth/Llama-3.2-1B-Instruct",
    "method": "qlora",
    "mode": "quick"
  }' | jq .
```

**Training modes:** `quick` (SFT only), `aligned` (SFT+DPO), `reasoning` (SFT+GRPO), `iterative` (multi-round)

**GPU class** (optional — defaults to Auto/A10G):
```bash
# For larger models, specify a GPU class:
  -d '{
    "dataset_id": "'$DATASET_ID'",
    "base_model": "unsloth/Meta-Llama-3.1-8B-Instruct",
    "method": "qlora",
    "mode": "aligned",
    "gpu_class": "a100"
  }'
# Options: t4, a10g, l40s, a10040gb, a10080gb, h100
```

### Monitor training

```bash
# Check job status:
JOB_ID=<from-response>
curl -s "$API/training-jobs/$JOB_ID" \
  -H "Authorization: Bearer $TOKEN" | jq .

# Stream live metrics (loss, learning rate, GPU utilization):
curl -N "$API/training-jobs/$JOB_ID/metrics/stream" \
  -H "Authorization: Bearer $TOKEN"

# Or watch in Temporal UI: http://localhost:8088
```

---

## 10. Deployment & Inference (Requires vLLM)

### Start vLLM (GPU Required)

Uncomment the vLLM section in `docker-compose.yml`, or run manually:

```bash
# Using Docker (needs NVIDIA Container Toolkit):
docker run --gpus all \
  -p 8080:8000 \
  vllm/vllm-openai:latest \
  --model meta-llama/Llama-3.1-8B-Instruct \
  --enable-lora \
  --max-lora-rank 64 \
  --max-loras 4 \
  --gpu-memory-utilization 0.85
```

### Deploy & Use

```bash
# Deploy model to vLLM
MODEL_ID=<model-id-from-training>
curl -s -X POST "$API/models/$MODEL_ID/deploy" \
  -H "Authorization: Bearer $TOKEN" | jq .

# Check deployment status
curl -s "$API/deployments/$MODEL_ID" \
  -H "Authorization: Bearer $TOKEN" | jq .

# Create API key for inference
curl -s -X POST "$API/models/$MODEL_ID/api-keys" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "test-key"}' | jq .
# → {"key": "pl_sk_...", ...}   ← Save this! Shown only once.

# Inference (OpenAI-compatible — note: /v1/ not /api/v1/)
curl -s -X POST http://localhost:8000/v1/chat/completions \
  -H "Authorization: Bearer pl_sk_your_key" \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "Hello!"}],
    "max_tokens": 256
  }' | jq .

# Streaming inference:
curl -N -X POST http://localhost:8000/v1/chat/completions \
  -H "Authorization: Bearer pl_sk_your_key" \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "Hello!"}],
    "max_tokens": 256,
    "stream": true
  }'
```

---

## 11. Exports (GGUF for Local Use)

```bash
# Export model as GGUF (for Ollama / llama.cpp)
curl -s -X POST "$API/exports" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model_id": "'$MODEL_ID'", "format": "gguf", "quantization": "Q4_K_M"}' | jq .

# List exports
curl -s "$API/exports" \
  -H "Authorization: Bearer $TOKEN" | jq .

# Download when ready
EXPORT_ID=<from-response>
curl -s "$API/exports/$EXPORT_ID/download" \
  -H "Authorization: Bearer $TOKEN" | jq .
# → Returns presigned S3 download URL
```

---

## 12. Deploy to Cloud

### Build Docker images

```bash
# Build all three images:
docker build -t braindrain-api -f crates/api/Dockerfile .          # ~20MB final
docker build -t braindrain-web -f apps/web/Dockerfile .            # ~50MB final
docker build -t braindrain-workers -f apps/workers/Dockerfile .    # ~500MB+ with ML deps
```

### Option A: VPS with Docker Compose (simplest)

```bash
# On your server (Hetzner, DigitalOcean, AWS EC2, etc.):
git clone <your-repo> /opt/platform
cd /opt/platform

# 1. Configure environment
cp .env.example .env
# Edit .env with production values:
#   - Strong passwords for DATABASE_URL, REDIS_URL
#   - Real Clerk keys (CLERK_SECRET_KEY, CLERK_JWKS_URL)
#   - Real Stripe keys (if billing enabled)
#   - Domain in CORS_ORIGINS
#   - LLM API key (APP_LLM_API_KEY)
#   - HuggingFace token (APP_HF_TOKEN)
#   - Platform internal token (PLATFORM_INTERNAL_TOKEN — generate a UUID)

# 2. Start everything
docker compose -f docker-compose.prod.yml build
docker compose -f docker-compose.prod.yml up -d

# 3. Verify
curl http://localhost:8000/health
curl http://localhost:3000
```

### Option B: Railway / Fly.io

Deploy templates are included in `.github/workflows/deploy-staging.yml` (commented out). Uncomment the matching section and configure:

1. Set `push: true` in the build jobs
2. Uncomment the registry login step
3. Uncomment the deploy job for your platform
4. Add secrets to your GitHub repo settings

### Production checklist

| Item | What to do |
|------|-----------|
| **Clerk** | Create production instance at clerk.com, get `pk_live_` and `sk_live_` keys |
| **Stripe** (optional) | Create 3 products/prices (Starter/Growth/Pro), configure webhook to `https://yourdomain.com/api/webhooks/stripe` |
| **Domain + TLS** | Point `yourdomain.com` → web, `api.yourdomain.com` → API. Use Caddy/Traefik for auto-TLS |
| **GPU training** | Set `APP_GPU_PROVIDER=modal` + Modal tokens, or run workers on a GPU VPS |
| **Monitoring** | Set `OTEL_ENABLED=true` for observability, or use Grafana Cloud |

> For the full deployment reference (env var checklist, cloud platform comparison, DNS/TLS setup), see [DEPLOYMENT.md](./DEPLOYMENT.md).

---

## Useful Commands

| Task | Command |
|------|---------|
| Start all infra | `make infra && make temporal` |
| Start observability (OTEL) | `make observability` |
| Stop all infra | `make infra-down` |
| Run migrations | `make migrate` |
| Start Rust API | `make dev-api` |
| Start frontend | `make dev-web` |
| Start workers | `make dev-workers` |
| Run all tests | `make test` |
| Lint everything | `make lint` |
| Regenerate TS types | `make typegen` |
| Release build | `make build` |
| Clean artifacts | `make clean` |

---

## Monitoring & Debugging

| What | Where |
|------|-------|
| API logs | Terminal running `make dev-api` |
| API docs (Swagger) | `http://localhost:8000/docs` |
| Worker logs | Terminal running `make dev-workers` |
| Temporal workflows | `http://localhost:8088` (Temporal UI) |
| MinIO files | `http://localhost:9001` (MinIO Console) |
| Database | `psql postgresql://platform:platform_dev@localhost:5432/platform` |
| Redis | `redis-cli -h localhost` |
| Grafana dashboards | `http://localhost:3001` (if observability stack running) |

---

## Shutting Down

```bash
make infra-down
# Stops all infrastructure:
#   docker compose down
#   docker compose -f infra/temporal/docker-compose.temporal.yml down
#   docker compose -f infra/otel/docker-compose.otel.yml down
```

---

*For architecture and flow details, see [PROJECT_FLOW.md](./PROJECT_FLOW.md). For the full deployment reference, see [DEPLOYMENT.md](./DEPLOYMENT.md).*
