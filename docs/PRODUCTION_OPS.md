# Production Operations Guide

Operational reference for running the platform in production.

---

## Connection Pooling (PgBouncer)

### Why PgBouncer?

In production, API replicas + workers + background tasks (relay, cleanup) all
open connections to PostgreSQL. Without pooling, each service maintains its own
connection pool, and the total connection count grows linearly with replicas.
PgBouncer sits between all services and Postgres, multiplexing many client
connections onto a smaller pool of real server connections.

### Architecture

```
API (20 conns) ─┐
Workers (10)  ──┼── PgBouncer (6432) ──── PostgreSQL (5432)
Relay (5)     ──┘   (pool_mode=transaction)
                    max_client_conn=200
                    default_pool_size=20
```

### Configuration

PgBouncer is configured in `infra/pgbouncer/pgbouncer.ini`. Key settings:

| Setting | Default | Description |
|---|---|---|
| `pool_mode` | `transaction` | Release server connection after each transaction |
| `default_pool_size` | 20 | Real Postgres connections per user/db pair |
| `max_client_conn` | 200 | Total client connections PgBouncer accepts |
| `min_pool_size` | 5 | Keep this many connections warm |
| `reserve_pool_size` | 5 | Extra connections for burst traffic |
| `server_reset_query` | `DEALLOCATE ALL` | Clean prepared statements between transactions |

Override via environment variables in `docker-compose.prod.yml`:

```bash
PGBOUNCER_DEFAULT_POOL_SIZE=30
PGBOUNCER_MAX_CLIENT_CONN=300
```

### Transaction mode compatibility

The platform uses `SET LOCAL` for tenant context (`app.tenant_id`), which
auto-reverts on transaction end — fully compatible with transaction-mode
pooling. Advisory locks used within transactions (billing relay, idempotency
cleanup) also work correctly because the lock is held for the transaction
duration.

**What does NOT work in transaction mode:**
- `SET` (without `LOCAL`) — state persists beyond the transaction
- Session-level prepared statements — use `DEALLOCATE ALL` reset query
- `LISTEN/NOTIFY` — requires persistent session

**Who connects directly to Postgres (bypasses PgBouncer):**
- The `migrate` service — DDL and migration advisory locks need session state

### Monitoring

Connect to the PgBouncer admin console:

```bash
psql -h pgbouncer -p 6432 -U pgbouncer_admin pgbouncer
```

Useful queries:
```sql
SHOW POOLS;     -- active/waiting/server connections per pool
SHOW CLIENTS;   -- connected clients
SHOW STATS;     -- query/transaction counts, latency
SHOW CONFIG;    -- current configuration
```

---

## Backup and Recovery (PITR)

### Overview

The platform uses PostgreSQL's WAL (Write-Ahead Log) archiving for
point-in-time recovery. Combined with periodic base backups, this enables
recovery to any moment between the oldest backup and the latest archived WAL.

### Setup

**Prerequisites:**
- [wal-g](https://github.com/wal-g/wal-g) installed on the Postgres host
- S3-compatible bucket for WAL storage
- AWS credentials with read/write access to the bucket

**Enable archiving:**
```bash
WAL_S3_BUCKET=my-wal-bucket \
WAL_S3_ENDPOINT=https://s3.amazonaws.com \
AWS_ACCESS_KEY_ID=... \
AWS_SECRET_ACCESS_KEY=... \
./infra/pitr/enable-wal-archiving.sh
```

Then restart PostgreSQL (wal_level change requires restart).

### Backup schedule

Run `infra/pitr/backup.sh` daily via cron:

```cron
0 3 * * * /opt/platform/infra/pitr/backup.sh >> /var/log/platform/backup.log 2>&1
```

This creates a full base backup and applies retention (keep last 7 backups).

### Verify archiver health

```bash
./infra/pitr/backup.sh --verify-only
```

Or query directly:
```sql
SELECT * FROM pg_stat_archiver;
-- Check: failed_count = 0, last_archived_time is recent
```

### Recovery procedure

**1. Stop PostgreSQL:**
```bash
systemctl stop postgresql
# or: docker compose -f docker-compose.prod.yml stop postgres
```

**2. Run restore:**
```bash
# Restore to specific time
./infra/pitr/restore.sh "2026-04-05 14:30:00+00"

# Or restore to latest available
./infra/pitr/restore.sh latest
```

**3. Start PostgreSQL** — it replays WAL to the target time:
```bash
systemctl start postgresql
```

**4. Verify, then re-enable archiving and take a fresh backup.**

---

## Release and Migration Pipeline

### Deploy order

The production compose enforces this dependency chain:

```
postgres (healthy) ──→ pgbouncer (healthy) ──→ api (healthy) ──→ web
                   └─→ migrate (completed) ──┤
                   └─→ temporal (healthy) ────┴─→ workers
```

1. **Infrastructure starts first:** Postgres, Redis, MinIO, Temporal
2. **PgBouncer** waits for Postgres healthy, then accepts connections
3. **Migrate** runs directly against Postgres (not PgBouncer), then exits
4. **API** starts only after migrate completes successfully
5. **Web** starts after API is healthy
6. **Workers** start after migrate completes, PgBouncer, Redis, and Temporal are healthy

### Migration safety

- Migrations run in a **dedicated container** that exits after completion
- The API **will not start** if migrations fail (`service_completed_successfully`)
- Migrations connect **directly to Postgres** (DDL needs session state)
- The migrate binary logs applied count before and after for auditability
- Billing partitions are verified/created at the end of every migration run

### Pre-deploy checklist

Run before deploying:

```bash
DATABASE_URL=postgres://... ./infra/release/pre-deploy-check.sh
```

This verifies:
- Database connectivity
- No failed migrations
- Billing partitions exist
- WAL archiving status (if configured)
- PgBouncer reachability
- Billing outbox health (no stuck rows)

### Rolling deploy (VPS/Docker Compose)

```bash
# 1. Pull new images
docker compose -f docker-compose.prod.yml pull

# 2. Run migrations first (direct to Postgres)
docker compose -f docker-compose.prod.yml run --rm migrate

# 3. Restart API, verify health, then restart others
docker compose -f docker-compose.prod.yml up -d --no-deps api
curl -sf http://localhost:8000/health

docker compose -f docker-compose.prod.yml up -d --no-deps web
docker compose -f docker-compose.prod.yml up -d --no-deps workers
```

### Rollback guidance

**If a migration fails:**
- The migrate container exits with non-zero status
- The API will not start (depends_on `service_completed_successfully`)
- Fix the migration, rebuild the image, and retry

**If a bad migration was already applied:**
- Write a new forward migration that undoes the change
- Never modify an already-applied migration file
- For emergencies: restore from PITR backup to before the migration

**If the API is unhealthy after deploy:**
- Roll back to the previous image tag
- Investigate logs: `docker compose logs api`
- The billing outbox relay drains on shutdown, so no events are lost

---

## Environment Variables

### PgBouncer

| Variable | Default | Description |
|---|---|---|
| `PGBOUNCER_DEFAULT_POOL_SIZE` | 20 | Server connections per user/db |
| `PGBOUNCER_MAX_CLIENT_CONN` | 200 | Total client connections |
| `PGBOUNCER_MIN_POOL_SIZE` | 5 | Warm connections |
| `PGBOUNCER_RESERVE_POOL_SIZE` | 5 | Burst connections |
| `PGBOUNCER_ADMIN_PASSWORD` | (required) | Admin console password |

### PITR

| Variable | Default | Description |
|---|---|---|
| `WAL_S3_BUCKET` | (required) | S3 bucket for WAL archives |
| `WAL_S3_ENDPOINT` | https://s3.amazonaws.com | S3 endpoint URL |
| `WAL_S3_REGION` | us-east-1 | S3 region |
| `WAL_S3_PREFIX` | wal-archive | Key prefix in bucket |
