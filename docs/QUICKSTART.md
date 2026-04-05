# BrainDrain Quickstart

> Set up the platform locally, run the full stack, and exercise training, deployment, and inference using the final production architecture.

## Prerequisites

| Tool | Version |
|------|---------|
| Docker + Docker Compose | 24+ |
| Rust | 1.75+ |
| Node.js | 20+ |
| pnpm | 9+ |
| Python | 3.11+ |
| uv | Latest |

## 1. Start Local Infrastructure

From the repo root:

```bash
make infra
make temporal
```

This starts:
- PostgreSQL on `localhost:5432`
- Redis on `localhost:6379`
- MinIO on `localhost:9000` / console `localhost:9001`
- Temporal on `localhost:7233` / UI `localhost:8088`

## 2. Configure Environment

### API

```bash
cp .env.example .env
```

Important local variables:
- `DATABASE_URL`
- `REDIS_URL`
- `S3_ENDPOINT`
- `S3_ACCESS_KEY`
- `S3_SECRET_KEY`
- `S3_BUCKET`
- `TEMPORAL_HOST`
- `INFERENCE_BACKEND_TYPE`
- `INFERENCE_SERVER_URL`
- `FEATURE_FLAGS_PROVIDER`
- `FEATURE_FLAGS_JSON`

### Web

```bash
cp apps/web/.env.local.example apps/web/.env.local
```

Key local variables:
- `NEXT_PUBLIC_API_URL=http://localhost:8000`

### Workers

Create `apps/workers/.env`:

```bash
APP_TEMPORAL_ADDRESS=localhost:7233
APP_TEMPORAL_NAMESPACE=default
APP_TEMPORAL_TASK_QUEUE=ml-pipeline

APP_DATABASE_URL=postgresql://platform:platform_dev@localhost:5432/platform
APP_REDIS_URL=redis://localhost:6379

APP_S3_ENDPOINT=http://localhost:9000
APP_S3_ACCESS_KEY=minioadmin
APP_S3_SECRET_KEY=minioadmin
APP_S3_BUCKET=platform-dev

APP_PLATFORM_API_URL=http://localhost:8000

APP_LLM_API_BASE_URL=https://api.openai.com/v1
APP_LLM_API_KEY=
APP_LLM_MODEL=gpt-4o-mini

APP_HF_TOKEN=
APP_WORKER_MODE=all
```

## 3. Run Migrations

```bash
make migrate
```

Local dev/staging can auto-run some startup safety setup, but production uses the dedicated migration path. For local work, `make migrate` is still the clean way to initialize the database.

## 4. Start the Services

### API

```bash
make dev-api
```

API endpoints:
- API root: `http://localhost:8000`
- Health: `http://localhost:8000/health`
- Swagger UI: `http://localhost:8000/docs`

### Web

```bash
cd apps/web
pnpm install
pnpm dev
```

Frontend: `http://localhost:3000`

### Workers

```bash
cd apps/workers
uv sync
uv run python -m src.worker
```

## 5. Local Auth

For local API testing, use a dev bearer token:

```bash
TENANT_ID=$(python -c "import uuid; print(uuid.uuid4())")
TOKEN="dev_${TENANT_ID}_testuser"
API=http://localhost:8000/api/v1
```

## 6. Create a Project and Upload Documents

```bash
curl -s -X POST "$API/projects" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"My First Project","description":"Testing BrainDrain","task_type":"qa"}'
```

Upload a document:

```bash
curl -s -X POST "$API/projects/$PROJECT_ID/documents" \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@path/to/your/document.pdf"
```

Trigger parse:

```bash
curl -s -X POST "$API/projects/$PROJECT_ID/parse" \
  -H "Authorization: Bearer $TOKEN"
```

Check status:

```bash
curl -s "$API/projects/$PROJECT_ID/status" \
  -H "Authorization: Bearer $TOKEN"
```

## 7. Configure LLM Settings

Synthetic data generation and evaluation require an LLM provider.

```bash
curl -s -X PUT "$API/settings/llm" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "provider": "openai",
    "api_base_url": "https://api.openai.com/v1",
    "api_key": "sk-your-key",
    "model": "gpt-4o-mini",
    "max_tokens": 2000
  }'
```

## 8. Create a Training Job

Estimate cost:

```bash
curl -s -X POST "$API/training-jobs/estimate" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "dataset_id": "'"$DATASET_ID"'",
    "base_model": "meta-llama/Llama-3.1-8B-Instruct",
    "training_mode": "quick",
    "training_method": "qlora"
  }'
```

Create the job:

```bash
curl -s -X POST "$API/training-jobs" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "dataset_id": "'"$DATASET_ID"'",
    "base_model": "meta-llama/Llama-3.1-8B-Instruct",
    "training_mode": "quick",
    "training_method": "qlora"
  }'
```

## 9. Start an Inference Backend

The control plane supports pluggable inference backends. For local work, start one backend and point the API at it with:
- `INFERENCE_BACKEND_TYPE=vllm|tgi|sglang`
- `INFERENCE_SERVER_URL=http://localhost:...`

### Example: vLLM

```bash
docker run --gpus all \
  -p 8080:8000 \
  vllm/vllm-openai:latest \
  --model meta-llama/Llama-3.1-8B-Instruct \
  --enable-lora \
  --max-lora-rank 64 \
  --max-loras 4 \
  --gpu-memory-utilization 0.85
```

Set in `.env`:

```bash
INFERENCE_BACKEND_TYPE=vllm
INFERENCE_SERVER_URL=http://localhost:8080
```

## 10. Deploy and Call a Model

Deploy:

```bash
curl -s -X POST "$API/models/$MODEL_ID/deploy" \
  -H "Authorization: Bearer $TOKEN"
```

Create an API key:

```bash
curl -s -X POST "$API/models/$MODEL_ID/api-keys" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"test-key"}'
```

Inference:

```bash
curl -s -X POST http://localhost:8000/v1/chat/completions \
  -H "Authorization: Bearer pl_sk_your_key" \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role":"user","content":"Hello!"}],
    "max_tokens": 256
  }'
```

Streaming:

```bash
curl -N -X POST http://localhost:8000/v1/chat/completions \
  -H "Authorization: Bearer pl_sk_your_key" \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role":"user","content":"Hello!"}],
    "max_tokens": 256,
    "stream": true
  }'
```

## 11. Multi-Instance Inference Mode

When the feature flag is enabled, deployments are routed through the inference instance registry instead of the single global backend.

Enable:

```bash
FEATURE_FLAGS_JSON={"deployments.multi_instance.enabled":true}
```

Register an instance:

```bash
curl -s -X POST "$API/admin/inference-instances" \
  -H "Authorization: Bearer $OWNER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "gpu-a10g-1",
    "base_url": "http://gpu-a10g-1:8000",
    "backend_type": "vllm",
    "base_model": "meta-llama/Llama-3.1-8B-Instruct",
    "max_adapters": 4,
    "metadata": {"region":"local"}
  }'
```

Behavior:
- flag off: legacy single-backend path uses `INFERENCE_SERVER_URL`
- flag on + healthy registered instances: deploys claim capacity on a matching instance
- inference and undeploy route through the assigned instance

## 12. Export Models

```bash
curl -s -X POST "$API/exports" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model_id":"'"$MODEL_ID"'","format":"gguf","quantization":"Q4_K_M"}'
```

## 13. Production Notes

For production, use:
- `docker-compose.prod.yml`
- dedicated migration container
- PgBouncer
- PITR scripts in `infra/pitr`
- release checks in `infra/release`

See:
- [DEPLOYMENT.md](./DEPLOYMENT.md)
- [PRODUCTION_OPS.md](./PRODUCTION_OPS.md)
- [SYSTEM_ARCHITECTURE.md](./SYSTEM_ARCHITECTURE.md)
