# Billing Architecture

Technical reference for how billing works in the platform — from event capture
to reporting, cost estimation to payment integration.

---

## Overview

The platform bills tenants for three types of resource usage:

| Usage type | When it happens | What's measured |
|---|---|---|
| **Inference** | Every chat completion request | Prompt tokens, completion tokens, estimated USD cost |
| **Training** | GPU time during fine-tuning | GPU-seconds, GPU hourly rate, actual cost |
| **Deployment** | Loading a LoRA adapter | Event logged (zero-cost event for audit trail) |

All billing flows through a single pipeline:

```
Usage event → Outbox (durable) → Relay → Billing ledger → Dashboards / Invoices
```

---

## Data Model

### billing_outbox (write surface)

The **transactional outbox** — where billing events are first written. This table
is the durability guarantee. If it's on disk, the event will eventually reach
the ledger.

```sql
billing_outbox
├── id              UUID (PK)
├── tenant_id       UUID
├── operation       TEXT ("inference", "deploy", "training", etc.)
├── resource_id     UUID (model ID, job ID, etc.)
├── tokens_in       BIGINT
├── tokens_out      BIGINT
├── gpu_seconds     INT
├── cost_usd        DECIMAL(10,4) (CHECK >= 0)
├── metadata        JSONB (api_key_id, batch info, etc.)
├── created_at      TIMESTAMPTZ
├── delivered_at    TIMESTAMPTZ (NULL until relay delivers)
├── attempt_count   INT (retry tracking)
└── last_error      TEXT (last delivery failure message)
```

### billing_events (reporting ledger)

The **source of truth** for dashboards, usage APIs, invoices, and analytics.
Monthly-partitioned for query performance. Append-only — never updated or deleted
in normal operation.

```sql
billing_events (PARTITIONED BY RANGE created_at)
├── id              UUID (PK)
├── tenant_id       UUID
├── operation       TEXT
├── resource_id     UUID
├── tokens_in       BIGINT
├── tokens_out      BIGINT
├── gpu_seconds     INT
├── cost_usd        DOUBLE PRECISION
├── metadata        JSONB
└── created_at      TIMESTAMPTZ
```

Partitions are created automatically:
- On startup: current month + 3 months ahead
- Hourly: the billing batcher's flush loop checks and creates missing partitions
- Without partitions, inserts into future months fail

### Why two tables?

| Concern | billing_outbox | billing_events |
|---|---|---|
| **Purpose** | Durability / transport | Reporting / analytics |
| **Who writes** | API handlers | Relay worker only |
| **Who reads** | Relay worker only | Dashboard, billing API, admin |
| **Retention** | 7 days after delivery | Permanent (partitioned) |
| **Partitioned** | No | Yes (monthly) |
| **Indexes** | Pending rows, delivered rows | Tenant + date, operation type |

Separating write surface from read surface means:
- Handler writes are simple single-table INSERTs (fast, no partition logic)
- Reporting queries never contend with high-frequency writes
- If the ledger has issues (missing partition, index bloat), the outbox buffers safely
- The relay delivers rows asynchronously in batches of up to 500, decoupling request handling from ledger writes

---

## Write Path

### How a billing event gets created

Every billing-producing handler calls `AppState::record_billing_event()`:

```rust
state.record_billing_event(
    tenant_id,
    "inference",
    Some(model_id),
    tokens_in,       // prompt tokens
    tokens_out,      // completion tokens
    0,               // gpu_seconds (0 for inference)
    estimated_cost,  // USD
    json!({"api_key_id": key_id}),
);
```

This is the **single entry point** for all billing writes. It routes to one of
two backends based on the `billing.outbox.enabled` feature flag:

```
record_billing_event()
├── Flag ON  → INSERT INTO billing_outbox (durable, crash-safe)
└── Flag OFF → Send to in-memory batcher channel (fast, lossy)
```

### Billing producers

| Producer | File | What it bills |
|---|---|---|
| Single inference | `routes/inference.rs` | Per-request token usage |
| Streaming inference | `routes/inference.rs` | Token usage from SSE final chunk |
| Batch inference | `routes/inference.rs` | Aggregated batch token usage |
| Model deployment | `services/deployment_service.rs` | Deploy event (via billing_event_repo) |
| Training completion | `workers/activities/train_model.py` | GPU-seconds × hourly rate |
| Training failure | `workers/activities/train_model.py` | Partial billing for long failures |

---

## Relay (Outbox → Ledger)

### How it works

The `BillingOutboxRelay` is a background worker that runs alongside the API server:

```
┌─────────────────────────────────────────────────────┐
│                    Relay Loop                        │
│                                                      │
│  1. pg_try_advisory_lock(900_200_001)               │
│     └── If locked by another instance → skip         │
│                                                      │
│  2. SELECT * FROM billing_outbox                     │
│     WHERE delivered_at IS NULL                        │
│       AND attempt_count < 5                           │
│     ORDER BY created_at                               │
│     LIMIT 500                                         │
│     FOR UPDATE SKIP LOCKED                            │
│                                                      │
│  3. For each row:                                     │
│     INSERT INTO billing_events (...)                  │
│                                                      │
│  4. UPDATE billing_outbox                             │
│     SET delivered_at = NOW()                          │
│     WHERE id = ANY(delivered_ids)                     │
│                                                      │
│  5. Failed rows: attempt_count++, last_error = ...   │
│                                                      │
│  6. pg_advisory_unlock(900_200_001)                  │
│                                                      │
│  Sleep(flush_interval) → repeat                      │
└─────────────────────────────────────────────────────┘
```

### Key design decisions

**`FOR UPDATE SKIP LOCKED`** — The heart of multi-instance safety. When multiple
API replicas run their own relay workers:
- `FOR UPDATE` locks claimed rows so no other worker can pick them up
- `SKIP LOCKED` skips rows already locked by another worker instead of blocking
- Result: parallel processing without duplication or contention

**Advisory lock** — `pg_try_advisory_lock(900_200_001)` ensures only one relay
instance processes at a time. This is a performance optimization (avoids redundant
`SELECT ... SKIP LOCKED` queries on every instance), not a correctness requirement.

**Retry with backoff** — Failed deliveries increment `attempt_count`. After 5
failed attempts, the row is left in the outbox for manual investigation. The
`last_error` column records what went wrong.

**Graceful shutdown** — On SIGTERM, the relay processes one final batch before
exiting. No events are lost between the shutdown signal and process exit.

---

## The Durability Guarantee

This is the core property that makes the outbox pattern valuable:

```
1. Handler starts
2. Business logic executes (inference, deploy, etc.)
3. INSERT INTO billing_outbox  →  PostgreSQL commits to WAL (on disk)
4. Handler returns 200 to client
5. ...later...
6. Relay picks up the row
7. INSERT INTO billing_events
8. UPDATE billing_outbox SET delivered_at = NOW()
```

**If the process crashes at any point:**

| Crash point | What happens | Event lost? |
|---|---|---|
| Before step 3 | INSERT never committed | No event to lose — user gets error |
| After step 3, before step 4 | Row is in outbox, user may retry | No — relay delivers it |
| After step 4, before step 7 | Row is in outbox, user got success | No — relay delivers it |
| After step 7, before step 8 | Delivered but not marked | No — relay re-delivers (idempotent) |

**The only way to lose a billing event is if PostgreSQL itself loses committed data**
— which is what PITR/WAL archiving protects against (see `docs/PRODUCTION_SCALE.md`).

### Comparison: in-memory batcher vs outbox

| Property | In-memory batcher | Outbox |
|---|---|---|
| **Write latency** | ~0 (channel send) | ~1ms (DB INSERT) |
| **Crash safety** | Events in channel are lost | Events survive any crash |
| **Multi-instance** | Each instance has own channel | Shared table with SKIP LOCKED |
| **Retry** | Best-effort, 3 batch retries | Per-row retry, 5 attempts |
| **Throughput** | ~10K events/sec (channel bound) | ~5K events/sec (DB bound) |
| **When to use** | Dev/staging, non-critical metrics | Production billing, financial data |

The feature flag lets you choose: `billing.outbox.enabled = true` for production
(durable), `false` for development (fast).

---

## Cost Estimation

### Inference pricing

Token-based pricing using the model:

```
input_cost  = tokens_in  × $0.15 / 1,000,000
output_cost = tokens_out × $0.60 / 1,000,000
total_cost  = input_cost + output_cost
```

Implemented in `services/token_estimator.rs`. These rates are configurable
and will eventually be per-plan.

### Training pricing

GPU-time pricing using rates from `shared/constants.rs`:

```rust
GPU_HOURLY_RATES = {
    "t4":       $0.80/hr,
    "a10g":     $1.20/hr,
    "l40s":     $1.80/hr,
    "a10040gb": $2.00/hr,
    "a10080gb": $3.00/hr,
    "h100":     $4.50/hr,
}
```

```
training_cost = gpu_seconds × (hourly_rate / 3600)
```

The Python worker calculates actual cost on training completion and writes it
via the platform API callback. Failed training jobs that ran for significant
time (> minimum billable threshold) are still billed at the elapsed GPU rate.

### Deployment pricing

Currently zero-cost — deploy events are logged for audit trail only. Future:
per-hour adapter hosting charges.

---

## Payment Integration (Stripe)

Billing events feed into Stripe for actual payment:

```
billing_events → Usage aggregation → Stripe metered billing
                                   → Stripe checkout sessions
                                   → Stripe customer portal
```

### Plan tiers

| Plan | Price ID env var | Limits |
|---|---|---|
| Starter | `STRIPE_PRICE_STARTER` | Basic usage caps |
| Growth | `STRIPE_PRICE_GROWTH` | Higher caps, priority |
| Pro | `STRIPE_PRICE_PRO` | Unlimited, SLA |

### Provider abstraction

The `BillingProvider` trait (`services/billing_provider.rs`) abstracts Stripe:

```rust
pub trait BillingProvider: Send + Sync {
    async fn create_checkout_session(...) -> Result<CheckoutSession>;
    async fn create_portal_session(...) -> Result<PortalSession>;
    async fn get_subscription(...) -> Result<Option<Subscription>>;
}
```

Two implementations:
- `StripeBillingProvider` — real Stripe API calls (production)
- `NoOpBillingProvider` — returns mock data (development, when `STRIPE_SECRET_KEY` is not set)

### Webhook handling

Stripe sends events to `/api/webhooks/stripe`:
- `checkout.session.completed` — activate subscription
- `customer.subscription.updated` — plan changes
- `customer.subscription.deleted` — cancellation

Webhooks use HMAC-SHA256 signature verification (not JWT auth) and advisory
locking for multi-instance idempotency.

---

## Observability

### Key metrics to monitor

| What to watch | Where | Alert threshold |
|---|---|---|
| Outbox pending count | `SELECT COUNT(*) FROM billing_outbox WHERE delivered_at IS NULL` | > 1000 |
| Outbox oldest pending | `SELECT MIN(created_at) FROM billing_outbox WHERE delivered_at IS NULL` | > 5 minutes old |
| Failed delivery count | `SELECT COUNT(*) FROM billing_outbox WHERE attempt_count >= 5` | > 0 |
| Billing events per hour | `billing_events` count by hour | Sudden drop = relay issue |
| Batcher channel capacity | Logs: "Billing batcher channel full" | Any occurrence |

### Troubleshooting

**Outbox rows not being delivered:**
1. Check if relay is running: look for "Billing outbox relay batch processed" in logs
2. Check advisory lock: `SELECT * FROM pg_locks WHERE locktype = 'advisory'`
3. Check attempt counts: `SELECT id, attempt_count, last_error FROM billing_outbox WHERE delivered_at IS NULL`

**Missing billing partitions:**
1. Partitions are created on startup and hourly
2. Manual fix: `SELECT create_billing_partition('2026-05-01'::date)`

**Batcher channel full (in-memory path):**
1. Increase `BILLING_CHANNEL_CAPACITY` (default: 10,000)
2. Decrease `BILLING_FLUSH_INTERVAL_SECS` (default: 5)
3. Consider enabling the outbox for true durability

---

## Configuration

| Env var | Default | Description |
|---|---|---|
| `BILLING_CHANNEL_CAPACITY` | 10,000 | In-memory batcher channel size |
| `BILLING_BATCH_SIZE` | 1,000 | Batcher flush threshold |
| `BILLING_FLUSH_INTERVAL_SECS` | 5 | Batcher/relay poll interval |
| `STRIPE_SECRET_KEY` | (none) | Stripe API key (enables real billing) |
| `STRIPE_WEBHOOK_SECRET` | (none) | Stripe webhook signature secret |
| `STRIPE_PRICE_STARTER` | (none) | Stripe price ID for starter plan |
| `STRIPE_PRICE_GROWTH` | (none) | Stripe price ID for growth plan |
| `STRIPE_PRICE_PRO` | (none) | Stripe price ID for pro plan |

Feature flag: `billing.outbox.enabled` — when `true`, billing writes go to the
durable outbox table instead of the in-memory batcher.

---

## Architecture Diagram

```
                    ┌──────────────┐
                    │   Client     │
                    └──────┬───────┘
                           │ POST /v1/chat/completions
                           ▼
                    ┌──────────────┐
                    │  API Handler │
                    │              │
                    │ 1. Process   │
                    │ 2. Bill      │──────────────────────────────┐
                    │ 3. Respond   │                              │
                    └──────────────┘                              │
                                                                  ▼
                    ┌─────────────────────────────────────────────────────┐
                    │           record_billing_event()                    │
                    │                                                     │
                    │  ┌─ Flag ON ──▶ INSERT INTO billing_outbox         │
                    │  │              (durable, crash-safe)               │
                    │  │                                                  │
                    │  └─ Flag OFF ─▶ mpsc channel → BillingBatcher      │
                    │                 (fast, in-memory)                   │
                    └─────────────────────────────────────────────────────┘
                                          │
                              ┌───────────┴───────────┐
                              ▼                       ▼
                    ┌──────────────┐        ┌──────────────┐
                    │   Outbox     │        │   Batcher    │
                    │   Relay      │        │   Flush      │
                    │              │        │              │
                    │ FOR UPDATE   │        │ Bulk INSERT  │
                    │ SKIP LOCKED  │        │ every 5s     │
                    └──────┬───────┘        └──────┬───────┘
                           │                       │
                           ▼                       ▼
                    ┌──────────────────────────────────────┐
                    │         billing_events               │
                    │     (monthly partitioned)             │
                    │                                       │
                    │  Source of truth for:                 │
                    │  • Dashboard usage stats              │
                    │  • GET /api/v1/billing                │
                    │  • Stripe metered billing             │
                    │  • Invoice generation                 │
                    └──────────────────────────────────────┘
```
