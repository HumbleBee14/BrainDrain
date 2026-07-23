# Deployment Checklist — From Local Dev to Cloud

This document tracks everything needed to make the platform 100% deployable and runnable on cloud infrastructure.

---

## 1. Dockerfiles (DONE)

All three services have multi-stage production Dockerfiles:

| Service | Dockerfile | Base Image | Final Size |
|---------|-----------|------------|------------|
| Rust API | `crates/api/Dockerfile` | rust:1.85-slim → debian:bookworm-slim | ~30MB |
| Python Workers | `apps/workers/Dockerfile` | python:3.11-slim (uv) | ~200MB |
| Next.js Frontend | `apps/web/Dockerfile` | node:20-alpine (standalone) | ~50MB |

All use non-root users, minimal runtime dependencies, and multi-stage builds.

---

## 2. CI/CD Pipeline (DONE)

### `.github/workflows/ci.yml` — Runs on every PR and push to main
- Rust: fmt check → clippy → tests (with Postgres service container)
- Frontend: pnpm install → lint → type-check
- Python: ruff check → ruff format check

### `.github/workflows/deploy-staging.yml` — Runs on push to main
- Builds all 3 Docker images with BuildKit + GHA cache
- `push: false` by design — builds validate Dockerfiles without pushing to a registry
- When ready for automated deployment: uncomment the registry login step, set `push: true`, and uncomment a deploy job template (Railway, Fly.io, or VPS via SSH)

---

## 3. .dockerignore (DONE)

Root `.dockerignore` excludes `target/`, `node_modules/`, `.venv/`, `.git/`, `docs/`, `infra/`, `.env` files, and IDE config. Prevents multi-GB Docker context on builds.

---

## 4. Production Docker Compose (DONE)

`docker-compose.prod.yml` runs the full stack:
- **Infrastructure:** Postgres 16, Redis 7, MinIO, Temporal (self-hosted with its own Postgres)
- **Application:** migrate job, API, Web, Workers (built from Dockerfiles)
- **Features:** health checks, restart policies, resource limits, volume persistence
- **Config:** all environment variables from `.env` via `${VAR}` interpolation
- **Optional:** vLLM inference server (uncomment for GPU hosts)

**Usage:**
```bash
cp .env.example .env          # Fill in real values
docker compose -f docker-compose.prod.yml build
docker compose -f docker-compose.prod.yml up -d
```

---

## 5. Environment Variables (DONE)

### `.env.example` — Master reference (all 3 services)
Covers every env var:
- API: database, Redis, S3, Clerk, Temporal, Stripe, CORS, rate limiting, OTEL
- Workers: `APP_` prefixed — Temporal, database, S3, LLM provider, HuggingFace, vLLM
- Frontend: `NEXT_PUBLIC_` — Clerk, API URL, app name

### Per-service `.env.example` files
- `apps/workers/.env.example` — worker-specific with explanations
- `apps/web/.env.local.example` — frontend-specific (already existed)

---

## 6. Registry Configuration (READY — Manual Control)

`deploy-staging.yml` has `push: false` intentionally — you control when to enable pushing:
1. Uncomment the registry login step in the workflow
2. Set `push: true` on each build job
3. Images will push to your configured registry (GHCR, Docker Hub, ECR)
4. Deploy templates (Railway, Fly.io, VPS) are included as commented blocks

---

## 7. Database Migrations (DONE)

Production uses a dedicated migration step before the API starts:

- `docker-compose.prod.yml` runs the `migrate` service first
- the API skips auto-migration in production
- the image still contains SQLx embedded migrations, so the same binary can run
  the migration job and the API process

For dev/staging, the API still auto-runs migrations on startup.

---

## 8. What Remains — Manual Setup Steps

These are things code can't automate — they require accounts, credentials, and decisions:

### 8a. Cloud Platform (CHOOSE ONE)

| Platform | Pros | Cons | Cost |
|----------|------|------|------|
| **VPS + Docker Compose** | Cheapest, full control, `docker-compose.prod.yml` ready to use | Manual scaling, manual TLS | ~$20-40/mo (Hetzner/DO) |
| **Railway** | Simplest DX, managed Postgres/Redis | No GPU, vendor lock-in | ~$20-50/mo |
| **Fly.io** | Global edge, great for Rust | More config than Railway | ~$20-50/mo |
| **AWS (ECS)** | GPU available, infinite scale | Most complex setup | ~$50-200/mo |

**For the VPS option**, the full deploy flow is:
```bash
# On server
git clone <repo> /opt/platform
cd /opt/platform
cp .env.example .env    # Edit with production values
docker compose -f docker-compose.prod.yml up -d
```

### 8b. External Service Accounts

| Service | What to do | Where |
|---------|-----------|-------|
| **Clerk** | Create production instance, get publishable + secret keys, configure domain | clerk.com |
| **Stripe** | Create account, create 3 products/prices (Starter/Growth/Pro), configure webhook URL pointing to `https://yourdomain.com/api/webhooks/stripe` | stripe.com |
| **LLM Provider** | Optional platform default — or let tenants bring their own (already built) | openai.com / groq.com |
| **HuggingFace** | Get token for model downloads during training | huggingface.co |
| **Domain** | Register domain, point DNS to server/platform | Any registrar |

### 8c. Production Secrets Checklist

Copy this and fill in each value:

```bash
# .env — Production values
ENVIRONMENT=production
DB_PASSWORD=<strong-random-password>
REDIS_PASSWORD=<strong-random-password>
S3_ACCESS_KEY=<generated>
S3_SECRET_KEY=<generated>
CLERK_SECRET_KEY=sk_live_...
CLERK_JWKS_URL=https://<your-clerk>.clerk.accounts.dev/.well-known/jwks.json
NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY=pk_live_...
STRIPE_SECRET_KEY=sk_live_...
STRIPE_WEBHOOK_SECRET=whsec_...
STRIPE_PRICE_STARTER=price_...
STRIPE_PRICE_GROWTH=price_...
STRIPE_PRICE_PRO=price_...
APP_LLM_API_KEY=sk-...  # Platform default LLM key
HF_TOKEN=hf_...         # For model downloads
PLATFORM_INTERNAL_TOKEN=<generated-uuid>  # Worker → API auth
CORS_ORIGINS=https://yourdomain.com
NEXT_PUBLIC_API_URL=https://api.yourdomain.com
```

### 8d. Domain & TLS

- Point `yourdomain.com` → frontend (web service)
- Point `api.yourdomain.com` → API service
- TLS: automatic via Caddy/Traefik reverse proxy, or platform-managed (Railway/Fly)
- Update `CORS_ORIGINS` env var with production domain
- Update Clerk dashboard "Allowed Origins" with production domain

---

## 9. GPU Compute Provider (DONE)

### Architecture

Training jobs dispatch through a **`GpuProvider` protocol** (`apps/workers/src/gpu_provider.py`) with pluggable implementations:

| Provider | Config | Use Case |
|----------|--------|----------|
| `LocalGpuProvider` | `APP_GPU_PROVIDER=local` (default) | Dev/testing on worker's own GPU |
| `ModalGpuProvider` | `APP_GPU_PROVIDER=modal` | Production — serverless GPUs (scale to zero, auto-scaling) |

### How it works

1. Worker reads `APP_GPU_PROVIDER` from config at startup
2. `create_gpu_provider()` factory returns the configured provider
3. `StartTrainingActivity` dispatches training through the provider
4. **Local**: runs `_run_training()` in-process (same behavior as before)
5. **Modal**: provisions an ephemeral GPU container, installs ML deps, runs training, returns results

### Local development (no changes needed)

```bash
# Default — uses your local GPU
APP_GPU_PROVIDER=local   # This is the default, you can omit it
```

### Production with Modal

```bash
# 1. Install modal
pip install modal

# 2. Set env vars
APP_GPU_PROVIDER=modal
MODAL_TOKEN_ID=ak-xxxxx      # From https://modal.com/settings
MODAL_TOKEN_SECRET=as-xxxxx

# 3. Deploy the training function: make modal-deploy
```

**Status:** the Modal path is validated for deploy and smoke-test training
runs (spawn/poll/reservation, adapter landing in S3). A full train→S3 run on
cloud infrastructure has a documented runbook but is not yet proven
end-to-end in this repo — see [CLOUD_GPU_TRAINING.md](./CLOUD_GPU_TRAINING.md#9-cheap-end-to-end-runbook-30-budget)
for the exact steps and known crash-window caveats. There is no RunPod (or
other third-party GPU marketplace) integration — `LocalGpuProvider` and
`ModalGpuProvider` are the only two implementations today.

### GPU types (Modal)

The `gpu_class` field maps to Modal GPU specifiers:
- `A10G` → cost-effective for 7B-13B LoRA (default)
- `A100` → larger models, faster training
- `A100-80GB` → 30B+ models
- `H100` → maximum throughput

### Adding a new provider (e.g., RunPod, Lambda)

1. Create a class implementing `GpuProvider` protocol in `gpu_provider.py`
2. Add it to `create_gpu_provider()` factory
3. Add config vars to `WorkerSettings`
4. Add optional dependency to `pyproject.toml`

---

## 9b. Inference Control Plane (DONE)

The serving layer is no longer tied to one process-global inference URL.

### What exists now

- `inference_instances` table tracks registered GPU serving nodes
- deployed models bind to `models.inference_instance_id` relationally
- deploy chooses a compatible healthy ready instance with spare adapter capacity
- inference and undeploy route to the assigned instance, not a single startup URL
- background health probes and adapter-count reconciliation repair drift
- admin API manages instance registration and lifecycle (`ready`, `draining`, `retired`)

### Why this matters

This is the seam that lets the platform grow from one GPU server to many
without another deploy/inference rewrite later.

### Future extensions intentionally left out of this PR

These are future operational expansions, not missing foundations:

- automatic GPU node provisioning
- service discovery / auto-registration
- Kubernetes operator
- cross-region scheduler
- global traffic steering
- autoscaling based on queue depth or latency

The current design leaves room for all of those later because the control plane
now tracks instances explicitly and routes via an assigned instance binding.

---

## 10. Observability (OPTIONAL)

### What exists:
- OTEL docker-compose (`infra/otel/docker-compose.otel.yml`) — Prometheus, Tempo, Loki, Grafana
- Config in API (`OTEL_ENABLED`, `OTEL_ENDPOINT`)
- Structured logging via `tracing` crate

### To enable:
1. Set `OTEL_ENABLED=true` in API env
2. Run the observability stack: `docker compose -f infra/otel/docker-compose.otel.yml up -d`
3. Access Grafana at `http://localhost:3001`

### For cloud:
- Grafana Cloud (free tier handles small deployments)
- Datadog or New Relic for managed observability

---

## Summary: What's Code vs. What's Setup

| Category | Status | Details |
|----------|--------|---------|
| Dockerfiles (3 services) | DONE | Multi-stage, non-root, minimal images |
| CI pipeline (lint/test) | DONE | Rust + Frontend + Python checks |
| CD pipeline (build/validate) | DONE | Build-only (`push: false`), deploy templates ready to uncomment |
| .dockerignore | DONE | Prevents bloated build context |
| Production compose | DONE | Full stack with health checks |
| Environment variables | DONE | Master + per-service .env.example files |
| Database migrations | DONE | Dedicated prod migration step, auto-run only in dev/staging |
| Cloud platform account | SETUP | Choose and create account |
| Clerk production keys | SETUP | Create production instance |
| Stripe configuration | SETUP | Products, prices, webhook URL |
| Domain + DNS + TLS | SETUP | Register, point, configure |
| GPU provider | DONE (Modal cloud path partially proven) | Trait-based: Local (default, fully exercised) + Modal (serverless, validated for deploy/smoke — full cloud train→S3 e2e not yet proven; no RunPod) |
| Model serving (vLLM/TGI/SGLang) + automated CD | CODE COMPLETE, NOT PROVEN E2E | Backend abstraction and CI build/validate pipeline exist; not yet exercised against sustained production traffic or a real automated deploy |

**Bottom line:** The code and configuration for every stage above is written and unit-tested. Two areas — the Modal cloud-GPU path and the vLLM/TGI/SGLang serving + CD path — are implemented but not yet proven end-to-end against real production traffic; treat those as "ready to validate," not "battle-tested." The remaining setup items are account creation and credential setup — things that require human decisions and credit cards.
