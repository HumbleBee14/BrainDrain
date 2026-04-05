# Production Setup Guide

Step-by-step guide to deploy the platform. Follow in order.

---

## Prerequisites

- A Linux server (Ubuntu 22.04+) with Docker and Docker Compose
- OR a PaaS account (Railway, Fly.io, Render)
- A domain name with DNS access
- 4GB+ RAM, 2+ vCPUs minimum (8GB/4vCPU recommended)

---

## Step 1: Generate Secrets

Every secret must be unique and cryptographically random.

```bash
echo "DB_PASSWORD=$(openssl rand -base64 32)"
echo "REDIS_PASSWORD=$(openssl rand -base64 32)"
echo "S3_ACCESS_KEY=$(openssl rand -hex 20)"
echo "S3_SECRET_KEY=$(openssl rand -base64 32)"
echo "TEMPORAL_DB_PASSWORD=$(openssl rand -base64 32)"
echo "PLATFORM_INTERNAL_TOKEN=$(openssl rand -hex 32)"
echo "PGBOUNCER_ADMIN_PASSWORD=$(openssl rand -base64 32)"
```

Save these in a password manager.

**Important:** `PLATFORM_INTERNAL_TOKEN` must be identical on API and Worker services (authenticates worker-to-API callbacks).

---

## Step 2: Set Up External Services

### 2a. Clerk (Authentication)

1. Go to [clerk.com](https://clerk.com) and create a **production** instance
2. Note: **Publishable Key** → `NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY`, **Secret Key** → `CLERK_SECRET_KEY`, **JWKS URL** → `CLERK_JWKS_URL`
3. Under **Domains**, add your production domain
4. Under **Allowed Origins**, add your frontend URL

### 2b. Stripe (Billing — Optional)

Skip if you don't need billing yet.

1. Create products with recurring prices: Starter, Growth, Pro → note `price_xxx` IDs
2. Get **Secret Key** → `STRIPE_SECRET_KEY`
3. Create webhook at `https://api.yourdomain.com/api/webhooks/stripe`
4. Note **Webhook Signing Secret** → `STRIPE_WEBHOOK_SECRET`

### 2c. LLM Provider (Synthetic Data Generation)

1. Get an API key from OpenAI, Groq, or any OpenAI-compatible provider
2. Set `APP_LLM_API_KEY`, `APP_LLM_API_BASE_URL`, `APP_LLM_MODEL`

### 2d. HuggingFace (Optional)

Only needed for gated models (Llama, Mistral). Get a token with `read` access → `HF_TOKEN`.

---

## Step 3: Configure Domain & TLS

### Caddy Reverse Proxy (Recommended for VPS)

```
app.yourdomain.com {
    reverse_proxy web:3000
}

api.yourdomain.com {
    reverse_proxy api:8000
}
```

### DNS Records

```
A     app.yourdomain.com    → <server-ip>
A     api.yourdomain.com    → <server-ip>
```

---

## Step 4: Create the .env File

```bash
cd /opt/platform
cp .env.example .env
nano .env
```

Required production values:

```bash
# Core
ENVIRONMENT=production
APP_NAME=YourAppName

# Secrets (from Step 1)
DB_PASSWORD=<generated>
REDIS_PASSWORD=<generated>
S3_ACCESS_KEY=<generated>
S3_SECRET_KEY=<generated>
TEMPORAL_DB_PASSWORD=<generated>
PLATFORM_INTERNAL_TOKEN=<generated>
PGBOUNCER_ADMIN_PASSWORD=<generated>

# Domain
CORS_ORIGINS=https://app.yourdomain.com
NEXT_PUBLIC_API_URL=https://api.yourdomain.com

# Clerk
CLERK_SECRET_KEY=sk_live_xxxx
CLERK_JWKS_URL=https://your-instance.clerk.accounts.dev/.well-known/jwks.json
NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY=pk_live_xxxx

# Feature flags (required for production)
FEATURE_FLAGS_PROVIDER=static
FEATURE_FLAGS_JSON={"billing.outbox.enabled":true,"idempotency.enforced":true,"deployments.multi_instance.enabled":false}

# LLM
APP_LLM_API_KEY=sk-xxxx
APP_LLM_API_BASE_URL=https://api.openai.com/v1
APP_LLM_MODEL=gpt-4o-mini
```

Note: `billing.outbox.enabled=true` is **required** in production — the API refuses to start without it.

---

## Step 5: Deploy

### Initial Setup

```bash
git clone <repo-url> /opt/platform
cd /opt/platform
cp .env.example .env
nano .env   # Fill production values

# Build all images
docker compose -f docker-compose.prod.yml build

# Start the stack
docker compose -f docker-compose.prod.yml up -d
```

### Service startup order (enforced by compose)

```
postgres → pgbouncer → api → web
       └→ migrate ──┘
       └→ temporal → workers
```

1. **Postgres** starts and becomes healthy
2. **PgBouncer** connects to Postgres, accepts connections on port 6432
3. **Migrate** runs directly against Postgres (bypasses PgBouncer), then exits
4. **API** starts only after migrations complete successfully
5. **Web** starts after API is healthy
6. **Workers** start after migrations complete, PgBouncer, Redis, and Temporal are healthy

### Verify

```bash
docker compose -f docker-compose.prod.yml ps
```

Expected — all services healthy:

```
NAME                STATUS
postgres            Up (healthy)
pgbouncer           Up (healthy)
redis               Up (healthy)
minio               Up (healthy)
temporal            Up (healthy)
migrate             Exited (0)
api                 Up (healthy)
web                 Up
workers             Up
```

---

## Step 6: Verify the Stack

### Health check

```bash
curl https://api.yourdomain.com/health
# Expected: {"status":"ok"}
```

### Pre-deploy check (run before future deployments)

```bash
DATABASE_URL=postgres://platform:$DB_PASSWORD@localhost:5432/platform \
  ./infra/release/pre-deploy-check.sh
```

### Frontend

Open `https://app.yourdomain.com` — you should see the sign-in page.

### Upload test

1. Create a project
2. Upload a PDF
3. Click "Parse Documents" — status should go `uploaded` → `parsing` → `parsed`

---

## Step 7: Production Hardening

### 7a. PITR Backup (Point-in-Time Recovery)

Set up WAL archiving for disaster recovery:

```bash
# On the Postgres host/container:
WAL_S3_BUCKET=my-wal-bucket \
AWS_ACCESS_KEY_ID=... \
AWS_SECRET_ACCESS_KEY=... \
./infra/pitr/enable-wal-archiving.sh

# Then restart Postgres (wal_level change requires restart)
docker compose -f docker-compose.prod.yml restart postgres

# Schedule daily base backups via cron:
0 3 * * * /opt/platform/infra/pitr/backup.sh >> /var/log/platform/backup.log 2>&1
```

### 7b. PgBouncer Tuning

PgBouncer runs in transaction mode by default. Tune pool sizes for your workload:

```bash
# In .env
PGBOUNCER_DEFAULT_POOL_SIZE=30    # Real Postgres connections
PGBOUNCER_MAX_CLIENT_CONN=300     # Total client connections accepted
```

Monitor via admin console:

```bash
PGPASSWORD=$PGBOUNCER_ADMIN_PASSWORD psql -h pgbouncer -p 6432 -U pgbouncer_admin pgbouncer -c "SHOW POOLS;"
```

### 7c. Feature Flags (Remote Management — Optional)

For kill switches without redeploy, use Unleash:

```bash
FEATURE_FLAGS_PROVIDER=unleash
UNLEASH_URL=http://unleash:4242
UNLEASH_API_TOKEN=your-client-token
```

Static provider (`FEATURE_FLAGS_JSON`) works fine without Unleash.

### 7d. Observability

```bash
OTEL_ENABLED=true
OTEL_ENDPOINT=http://otel-collector:4317

# Start the observability stack
docker compose -f infra/otel/docker-compose.otel.yml up -d
```

Grafana at port 3001 with dashboards for API latency, worker activity, DB connections.

### 7e. Security Checklist

- [ ] `ENVIRONMENT=production`
- [ ] `CORS_ORIGINS` set to exact domain (not `*`)
- [ ] `REDIS_PASSWORD` is set (required in production compose)
- [ ] `PGBOUNCER_ADMIN_PASSWORD` is set (required)
- [ ] `billing.outbox.enabled=true` in feature flags
- [ ] `PLATFORM_INTERNAL_TOKEN` matches on API + Workers
- [ ] All secrets are strong random values
- [ ] TLS/HTTPS active on both domains
- [ ] `.env` is `chmod 600` and not in git
- [ ] Server firewall: only ports 80, 443, 22

---

## Updating

```bash
cd /opt/platform
git pull origin main

# Rebuild changed images
docker compose -f docker-compose.prod.yml build

# Run migrations first (direct to Postgres)
docker compose -f docker-compose.prod.yml run --rm migrate

# Rolling restart: API first, verify, then rest
docker compose -f docker-compose.prod.yml up -d --no-deps api
curl -sf https://api.yourdomain.com/health

docker compose -f docker-compose.prod.yml up -d --no-deps web
docker compose -f docker-compose.prod.yml up -d --no-deps workers
```

---

## Multi-Instance Inference (When You Need Multiple GPUs)

When you're ready for multiple GPU servers:

1. Enable the feature flag: `"deployments.multi_instance.enabled": true`
2. Register instances via the admin API:

```bash
curl -X POST https://api.yourdomain.com/api/v1/admin/inference-instances \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "gpu-a10g-1",
    "base_url": "http://vllm-1:8080",
    "backend_type": "vllm",
    "gpu_class": "a10g",
    "base_model": "meta-llama/Llama-3.1-8B",
    "max_adapters": 4
  }'
```

3. Deploy models normally — the platform routes to instances with capacity
4. To drain an instance for maintenance:

```bash
curl -X PUT .../admin/inference-instances/{id}/lifecycle \
  -d '{"lifecycle_state": "draining"}'
# New deployments go elsewhere; existing inference continues
```

Without multi-instance enabled, everything routes to the single `INFERENCE_SERVER_URL` — no configuration change needed.

---

## Architecture Reference

```
Internet
   │
   ├── https://app.yourdomain.com → [Caddy/TLS] → web:3000 (Next.js)
   │
   └── https://api.yourdomain.com → [Caddy/TLS] → api:8000 (Rust/Axum)
                                                      │
                                        ┌─────────────┼──────────────┐
                                        ▼             ▼              ▼
                                   pgbouncer:6432  redis:6379   minio:9000
                                        │
                                   postgres:5432
                                        ▲
                                        │
                                   workers → temporal:7233
                                        │
                                        └→ LLM API (OpenAI/Groq)
                                        └→ GPU Servers (vLLM/TGI/SGLang)
```

**Key services:**
- **PgBouncer** — connection pooling (transaction mode, 6432)
- **Migrate** — runs once per deploy, exits after completion
- **API** — auth, CRUD, billing outbox, idempotency, inference routing
- **Workers** — document parsing, training, evaluation (Temporal orchestrated)
- **Health prober** — checks GPU instance health every 60s
- **Reconciler** — repairs adapter count drift every 60s

---

## Troubleshooting

### API won't start

```bash
docker compose -f docker-compose.prod.yml logs api --tail=30
```

- **"billing.outbox.enabled must be true in production"** → Set `FEATURE_FLAGS_JSON` with `"billing.outbox.enabled":true`
- **"Failed to connect to PostgreSQL"** → Check PgBouncer is healthy, `DB_PASSWORD` matches
- **"TGI backend selected but feature flag disabled"** → Enable `inference.backend.tgi.enabled` if using TGI

### Migrations failed

Migrations run in a dedicated container. API won't start until they pass:

```bash
docker compose -f docker-compose.prod.yml logs migrate
```

### Workers can't connect to Temporal

Temporal takes 30-60s to start. Workers retry automatically. Check:

```bash
docker compose -f docker-compose.prod.yml logs workers --tail=30
```

### PgBouncer healthcheck failing

```bash
docker compose -f docker-compose.prod.yml logs pgbouncer --tail=10
```

Check `PGBOUNCER_ADMIN_PASSWORD` is set and `DB_PASSWORD` matches Postgres.
