# Release Pipeline Hardening

**PR:** #24  
**Problem:** Deploying without a defined order can break things. If the API
starts before migrations run, it hits missing tables. If workers start before
Temporal is ready, they crash. If there's no health check, a broken deploy
goes unnoticed.

## Deploy order

The production compose enforces this dependency chain:

```
postgres ──→ pgbouncer ──→ api ──→ web
         └─→ migrate ────┘
         └─→ temporal ──→ workers
```

Each arrow means "wait until healthy/completed before starting."

## Migration safety

- **Dedicated container:** Migrations run in their own container, then exit.
- **Blocks API startup:** API depends on `migrate: service_completed_successfully`.
  If migration fails, API never starts — no partial state.
- **Direct Postgres connection:** Migrations bypass PgBouncer (DDL needs
  session-level state).
- **Audit logging:** The migrate binary logs how many migrations existed
  before and after, so you can verify in logs.
- **Billing partitions:** Automatically created/verified at the end of
  every migration run (current month + 3 months ahead).

## Pre-deploy check

Run before deploying:

```bash
DATABASE_URL=postgres://... ./infra/release/pre-deploy-check.sh
```

Checks: DB connectivity, no failed migrations, billing partitions, WAL
archiving status, PgBouncer health, billing outbox health.

## Rolling deploy

For VPS/Docker Compose:

```bash
docker compose pull                          # 1. Get new images
docker compose run --rm migrate              # 2. Migrations first
docker compose up -d --no-deps api           # 3. API restart
curl -sf http://localhost:8000/health        # 4. Verify health
docker compose up -d --no-deps web workers   # 5. Rest of stack
```

## Rollback

- **Bad migration:** Fix and re-run. API won't start until migration passes.
- **Applied bad migration:** Write a new forward migration to undo it.
  Never edit already-applied migration files.
- **Emergency:** Restore from PITR backup to before the migration.
- **Bad code:** Roll back to previous image tag, restart.

## Files

- `crates/db/src/migrate.rs` — Migration binary with logging
- `infra/release/pre-deploy-check.sh` — Pre-deploy verification
- `docker-compose.prod.yml` — Dependency chain
- `.github/workflows/deploy-staging.yml` — CI deploy template
