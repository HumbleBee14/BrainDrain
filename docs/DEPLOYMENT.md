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
- **Application:** API, Web, Workers (built from Dockerfiles)
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

The API binary runs migrations on startup (SQLx embedded migrations).
The Dockerfile copies `crates/db/src/migrations/` into the image.
No manual migration step needed.

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

## 9. Observability (OPTIONAL)

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
| Database migrations | DONE | Auto-run on API startup |
| Cloud platform account | SETUP | Choose and create account |
| Clerk production keys | SETUP | Create production instance |
| Stripe configuration | SETUP | Products, prices, webhook URL |
| Domain + DNS + TLS | SETUP | Register, point, configure |
| GPU provider (optional) | SETUP | Modal/RunPod for training |

**Bottom line:** All code and configuration is done. The remaining items are account creation and credential setup — things that require human decisions and credit cards.
