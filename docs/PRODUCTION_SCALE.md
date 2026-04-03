# Production Scale Guide

Operational playbook for running this platform at scale. Items here are
infrastructure-level concerns that live outside the application codebase.

---

## 1. Pluggable Inference Backends

The platform abstracts the LoRA serving engine behind an `InferenceBackend`
trait. Any OpenAI-compatible engine can be plugged in by setting two env vars:

| Engine  | `INFERENCE_BACKEND_TYPE` | Notes |
|---------|--------------------------|-------|
| vLLM    | `vllm` (default)         | Best dynamic LoRA API, widest model support |
| TGI     | `tgi`                    | HF's battle-tested engine, used by HF Inference API |
| SGLang  | `sglang`                 | Fastest for structured generation |

```bash
# docker-compose.prod.yml or .env
INFERENCE_BACKEND_TYPE=tgi
INFERENCE_SERVER_URL=http://tgi-server:8080
```

Running multiple engines in parallel is possible by deploying separate API
replicas, each with a different `INFERENCE_BACKEND_TYPE` and
`INFERENCE_SERVER_URL`, behind a load balancer or API gateway that routes by
model type.

---

## 2. Point-In-Time Recovery (PITR)

The platform's PostgreSQL instance must have WAL archiving enabled so you
can recover to any second, not just the last daily snapshot.

### Setup (self-hosted PostgreSQL 16)

```bash
# postgresql.conf
wal_level = replica
archive_mode = on
archive_command = 'aws s3 cp %p s3://your-wal-bucket/wal/%f'
# or for MinIO:
archive_command = 'mc cp %p myminio/wal-archive/%f'

max_wal_senders = 3
wal_keep_size = 1GB   # keep 1 GB locally before archiving
```

Verify archiving is working:
```sql
SELECT * FROM pg_stat_archiver;
-- last_failed_wal should be NULL
-- archived_count should be increasing
```

### Restore procedure

```bash
# 1. Stop the API (take it down to prevent writes)
docker compose -f docker-compose.prod.yml stop api workers

# 2. Restore base backup
aws s3 cp s3://your-backup-bucket/base/latest.tar.gz /tmp/
tar -xzf /tmp/latest.tar.gz -C /var/lib/postgresql/data/

# 3. Create recovery.conf (PG 16: postgresql.conf + recovery signal file)
cat >> /var/lib/postgresql/data/postgresql.conf <<EOF
restore_command = 'aws s3 cp s3://your-wal-bucket/wal/%f %p'
recovery_target_time = '2026-04-01 14:30:00 UTC'
recovery_target_action = promote
EOF
touch /var/lib/postgresql/data/recovery.signal

# 4. Start PostgreSQL — it will replay WAL up to the target time
docker compose -f docker-compose.prod.yml up -d postgres
# Watch logs until "database system is ready to accept connections"

# 5. Restart the application
docker compose -f docker-compose.prod.yml up -d api workers
```

### Managed options

- **RDS PostgreSQL / Aurora**: Enable "automated backups" + set backup
  retention to 7–35 days. PITR is point-and-click in the console.
- **Supabase**: PITR available on Pro plan.
- **Neon**: Branching gives instant PITR with no extra config.

### RTO / RPO targets

| Scenario | RPO | RTO |
|----------|-----|-----|
| WAL archiving to S3 | ~30s | ~15 min |
| RDS with PITR | ~5s | ~10 min |
| Aurora Global | ~1s | <1 min |

---

## 3. PgBouncer Connection Pooling

At scale (100+ API replicas), each pod opens `DATABASE_MAX_CONNECTIONS`
connections directly to Postgres. With the default of 20, 100 pods = 2000
connections — well above PostgreSQL's `max_connections` default of 100.

PgBouncer sits between the application and PostgreSQL, multiplexing thousands
of application connections onto a small pool of actual server connections.

### Docker Compose addition

```yaml
# docker-compose.prod.yml additions

pgbouncer:
  image: bitnami/pgbouncer:1.22
  restart: unless-stopped
  environment:
    POSTGRESQL_HOST: postgres
    POSTGRESQL_PORT: "5432"
    POSTGRESQL_DATABASE: platform
    POSTGRESQL_USERNAME: platform
    POSTGRESQL_PASSWORD: ${DB_PASSWORD}
    PGBOUNCER_DATABASE: platform
    PGBOUNCER_POOL_MODE: transaction        # Required for RLS + sqlx
    PGBOUNCER_MAX_CLIENT_CONN: "5000"
    PGBOUNCER_DEFAULT_POOL_SIZE: "25"       # Real PG connections per DB
    PGBOUNCER_MIN_POOL_SIZE: "5"
    PGBOUNCER_RESERVE_POOL_SIZE: "5"
    PGBOUNCER_SERVER_IDLE_TIMEOUT: "300"
    PGBOUNCER_LOG_CONNECTIONS: "0"
    PGBOUNCER_LOG_DISCONNECTIONS: "0"
  depends_on:
    postgres:
      condition: service_healthy
  healthcheck:
    test: ["CMD-SHELL", "psql -h localhost -p 5432 -U pgbouncer pgbouncer -c 'SHOW POOLS' -t -A"]
    interval: 10s
    timeout: 5s
    retries: 5
```

### Application change

Point `DATABASE_URL` at PgBouncer, not PostgreSQL directly:

```bash
# Before
DATABASE_URL=postgres://platform:${DB_PASSWORD}@postgres:5432/platform

# After
DATABASE_URL=postgres://platform:${DB_PASSWORD}@pgbouncer:5432/platform
```

### Important: Transaction pool mode and `SET LOCAL`

The platform uses `SET LOCAL app.tenant_id = $1` for Row-Level Security.
`SET LOCAL` is scoped to a transaction, which is safe in transaction pool mode
because PgBouncer returns the connection to the pool only when the transaction
commits/rolls back — the `SET LOCAL` is already gone.

The `before_acquire` hook in `crates/db/src/lib.rs` resets `app.tenant_id`
on every checkout as a defense-in-depth measure.

**Do not use session pool mode** — `SET LOCAL` state would survive connection
reuse and leak tenant context between requests.

### Scaling math

| API replicas | Connections/pod | Raw PG conns | PgBouncer server pool |
|-------------|----------------|--------------|----------------------|
| 10          | 20             | 200          | 25                   |
| 100         | 20             | 2000         | 25                   |
| 1000        | 20             | 20000        | 25–50                |

PgBouncer keeps PostgreSQL's `max_connections` at a safe ~100–150 regardless
of API replica count.

---

## 4. Serving Control Plane

For production multi-tenant deployments, the current single-vLLM architecture
has limits. This section documents the evolution path.

### Current architecture (Phase 1)

```
API → vLLM (single instance, --enable-lora, --max-loras 4)
```

- All tenants share one GPU
- Max 4 adapters loaded simultaneously
- No isolation between tenants

Suitable for: early access, <50 active deployments, single GPU node.

### Phase 2: Adapter registry + routing

```
API → Serving Router → vLLM Pool (N instances)
                     ↘ Adapter Registry (Redis)
```

**Serving Router** (lightweight Axum service):
- Maintains a registry of which adapter is loaded on which vLLM instance
- Routes inference requests to the instance holding the target adapter
- Evicts least-recently-used adapters when capacity is full (LRU)
- Health-checks vLLM instances and removes unhealthy ones from rotation

**Adapter Registry** (Redis Hash):
```
serving:adapters:{adapter_ref} → {instance_url, loaded_at, last_used}
serving:instances:{url} → {adapter_count, capacity, healthy}
```

Implementation: add a `RouterBackend` to the `InferenceBackend` trait
implementations that proxies through the serving router instead of a single
vLLM URL.

### Phase 3: Dedicated GPU per tenant (enterprise)

For SLA-guaranteed isolation:
- Each tenant gets a dedicated node pool
- Deployment writes node assignment to `models.deployment_config`
- Inference router sends traffic to the tenant's node
- Cost is billed per-node rather than per-token for dedicated tier

### GPU autoscaling

Use KEDA (Kubernetes Event-Driven Autoscaler) with a custom scaler that reads
from the `models` table:

```yaml
# keda-scaler.yaml
triggers:
  - type: postgresql
    metadata:
      query: >
        SELECT COUNT(*) FROM models
        WHERE deployment_status = 'active'
        AND tenant_id IN (SELECT id FROM tenants WHERE plan = 'pro')
      targetQueryValue: "2"  # 1 GPU node per 2 active pro deployments
```

---

## 5. Billing Durability

The current billing batcher writes to an in-memory channel then bulk-inserts
to `billing_events`. Events in the channel are lost on process crash.

### Current guarantees
- Normal operation: events batched and flushed every 5s ✓
- Channel full: synchronous direct insert via `spawn_blocking` ✓
- Batch flush failure: retry up to 3 times, then individual inserts ✓
- **Process crash**: events in the channel since last flush are lost ✗

### Planned: DB outbox
Add a `billing_outbox` table written transactionally alongside each business
operation. A background worker moves rows from `billing_outbox` to
`billing_events` using `SELECT FOR UPDATE SKIP LOCKED`. This achieves
exactly-once billing even through API restarts.

Migration sketch:
```sql
CREATE TABLE billing_outbox (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   UUID NOT NULL,
    operation   TEXT NOT NULL,
    resource_id UUID,
    tokens_in   BIGINT NOT NULL DEFAULT 0,
    tokens_out  BIGINT NOT NULL DEFAULT 0,
    gpu_seconds INT NOT NULL DEFAULT 0,
    cost_usd    DOUBLE PRECISION NOT NULL DEFAULT 0,
    metadata    JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX ON billing_outbox (created_at);
```

Until the outbox is implemented, the RPO for billing is the flush interval
(5 seconds by default, configurable via `BILLING_FLUSH_INTERVAL_SECS`).

---

## 6. Operational Runbook

### Health checks

```bash
# API
curl -f http://localhost:8000/health && echo OK

# Inference backend
curl -f ${INFERENCE_SERVER_URL}/health

# PostgreSQL
psql $DATABASE_URL -c "SELECT 1"

# Redis
redis-cli -u $REDIS_URL PING

# Temporal
temporal workflow list --namespace default
```

### Common alerts

| Alert | Cause | Action |
|-------|-------|--------|
| `billing_batcher_channel_full` | Spike in inference traffic | Increase `BILLING_CHANNEL_CAPACITY`, investigate request rate |
| `inference_circuit_breaker_open` | Inference backend unhealthy | Check backend logs, restart if needed |
| `billing_partition_missing` | Cron missed monthly partition creation | Run `SELECT create_billing_partition(...)` manually |
| `stale_deployments_reaped > 0` | API pods crashed mid-deploy | Normal during rolling deploys; investigate if persistent |

### Rolling deploy checklist

1. Run `make migrate` (or wait for the `migrate` container to complete)
2. Scale API to new version — old pods drain, new pods start
3. Verify `/health` returns 200 on at least one new pod
4. Monitor `inference_circuit_breaker_state` metric (should stay `closed`)
5. Check `billing_events` row count is still increasing

### Capacity planning

| Metric | Threshold | Action |
|--------|-----------|--------|
| PgBouncer client wait time > 10ms | Pool saturation | Increase `PGBOUNCER_DEFAULT_POOL_SIZE` |
| Redis memory > 80% | Cache pressure | Increase Redis `maxmemory` or add replica |
| vLLM GPU util > 85% | Inference saturation | Add GPU node or raise `VLLM_MAX_LORAS` |
| `billing_events` table > 100M rows | Partition bloat | Confirm monthly partitioning is running, archive old partitions |
