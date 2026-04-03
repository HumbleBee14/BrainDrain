# BrainDrain — Observability Architecture

How every request, workflow, training step, and failure is tracked across the platform.

---

## Overview

```
┌─────────────────┐     ┌─────────────────┐
│   Rust API      │     │  Python Workers  │
│  (Axum/Tokio)   │     │  (Temporal SDK)  │
│                 │     │                  │
│  tracing crate  │     │  logging module  │
│  OTEL SDK       │     │  OTEL SDK        │
└────────┬────────┘     └────────┬─────────┘
         │ gRPC (4317)           │ gRPC (4317)
         ▼                       ▼
┌─────────────────────────────────────────────┐
│           OTEL Collector                     │
│  (opentelemetry-collector-contrib)           │
│                                              │
│  Receives: traces, metrics, logs via OTLP    │
│  Processes: batching (5s / 1024), mem limit  │
│  Exports to:                                 │
│    traces  → Tempo (port 3200)               │
│    metrics → Prometheus (port 9090)           │
│    logs    → Loki (port 3100)                │
└──────────────────────┬──────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────┐
│              Grafana (port 3001)             │
│                                              │
│  Datasources: Prometheus, Tempo, Loki        │
│  Dashboards:  API, Training, Temporal        │
│  Alerts:      4 rules (error rate, latency,  │
│               queue backup, job failures)    │
└─────────────────────────────────────────────┘
```

---

## How to Enable

```bash
# 1. Start the observability stack
make observability
# Starts: OTEL Collector, Prometheus, Tempo, Loki, Grafana

# 2. Enable OTEL export in the API (.env)
OTEL_ENABLED=true
OTEL_ENDPOINT=http://localhost:4317

# 3. Enable OTEL export in the workers (apps/workers/.env)
APP_OTEL_ENABLED=true
APP_OTEL_ENDPOINT=http://localhost:4317

# 4. Open Grafana
# http://localhost:3001  (admin / admin)
```

All observability is **opt-in** via `OTEL_ENABLED`. When disabled (default), the
API and workers still log structured JSON but don't export traces or metrics.

---

## Three Pillars

### 1. Logging

| Service | Library | Format | Config |
|---------|---------|--------|--------|
| Rust API | `tracing` + `tracing-subscriber` | JSON (prod) / human-readable (dev) | `LOG_LEVEL`, `ENVIRONMENT` |
| Python Workers | `logging` + `python-json-logger` | JSON (prod) / text (dev) | `APP_LOG_LEVEL`, `APP_LOG_FORMAT` |

**How it works (API)**:
```
init_tracing() in main.rs
  ↓
EnvFilter reads LOG_LEVEL (default: debug)
  ↓
if ENVIRONMENT == "development":
    fmt::layer().with_target(true)      ← human-readable
else:
    fmt::layer().json()                 ← structured JSON for Loki
  ↓
if OTEL_ENABLED:
    tracing-opentelemetry layer added   ← bridges spans to OTEL
```

**How it works (Workers)**:
```
setup_logging() in worker.py
  ↓
if APP_LOG_FORMAT == "text":
    logging.Formatter("%(asctime)s %(levelname)s ...")
else:
    JsonFormatter(...)                   ← structured JSON for Loki
  ↓
if APP_OTEL_ENABLED:
    LoggingInstrumentor().instrument()   ← injects trace_id/span_id into logs
```

**Log locations** (93 log calls in workers, 59 in API services):
- Every activity start/end, every workflow stage, every S3 upload/download
- All errors with context (document_id, job_id, tenant_id)
- Circuit breaker state changes, rate limit hits, auth failures

**Request ID propagation**:
- `X-Request-Id` header set on every API request (UUID v4)
- Propagated through all log entries via `tower-http` middleware
- Allows tracing a single request across all log lines

### 2. Traces

| Service | Library | Integration |
|---------|---------|-------------|
| Rust API | `tracing-opentelemetry` | Bridges Rust `tracing::span!()` to OTEL spans |
| Python Workers | `temporalio.contrib.opentelemetry.TracingInterceptor` | Auto-traces all workflow and activity executions |

**What gets traced**:

```
API Request (Axum TraceLayer)
├── Auth (JWT verification / dev token parse)
├── Service call (e.g. PipelineService::trigger_parse)
│   ├── DB query (repository method)
│   └── Temporal workflow start
└── Response serialization

Temporal Workflow (TracingInterceptor)
├── IngestWorkflow.run
│   ├── get_document_info activity
│   ├── parse_document activity
│   │   ├── S3 download
│   │   ├── PDF extraction
│   │   ├── Language detection
│   │   └── S3 upload
│   └── (next document...)
├── RefineWorkflow.run
│   ├── chunk_text activity
│   ├── generate_synthetic_pairs activity
│   │   └── LLM API calls (per chunk)
│   └── build_dataset activity
└── TrainWorkflow.run
    └── start_training activity
        ├── Model loading
        ├── SFT training (with per-step metrics)
        └── Adapter upload to S3
```

**Trace export path**:
```
tracing::span!() / activity execution
  → OTEL SDK BatchSpanProcessor
  → OTLP gRPC exporter (port 4317)
  → OTEL Collector
  → Tempo (trace storage)
  → Grafana (visualization + search)
```

### 3. Metrics

| Metric | Type | Source | Description |
|--------|------|--------|-------------|
| `http_server_request_duration_seconds` | Histogram | API middleware | Request latency by route, method, status |
| `http_server_requests_total` | Counter | API middleware | Total request count by route, method, status |
| Training step metrics | Redis Stream | Worker callback | loss, learning_rate, grad_norm, GPU util, ETA |

**HTTP metrics** are emitted by `HttpMetrics` middleware in `middleware.rs`:
```rust
// On every request completion:
metrics.request_duration.record(duration_secs, &[
    KeyValue::new("http.method", method),
    KeyValue::new("http.route", route),
    KeyValue::new("http.status_code", status),
]);
metrics.request_counter.add(1, &attrs);
```

**Training metrics** are streamed via Redis (not OTEL) for real-time dashboard:
```python
# MetricsStreamingCallback.on_log() — every training step:
redis.xadd("training:metrics:{job_id}", {
    "step": "42",
    "total_steps": "1000",
    "loss": "0.234",
    "learning_rate": "0.0002",
    "eta_seconds": "3600",
    "gpu_utilization": "95",
    "gpu_memory_pct": "78.3",
    "timestamp": "2026-04-03T10:15:30Z",
})
```

Clients consume these via:
- **WebSocket**: `ws://api/ws?token=...` → subscribe to `training:{job_id}`
- **SSE**: `GET /api/v1/training-jobs/{id}/metrics/stream`
- **REST**: `GET /api/v1/training-jobs/{id}/metrics` (latest snapshot)

**Metrics export path**:
```
OTEL SDK Meter
  → PeriodicReader (15s interval)
  → OTLP gRPC exporter
  → OTEL Collector
  → Prometheus (scrape endpoint :8889)
  → Grafana (dashboards + alerts)
```

---

## Grafana Dashboards

### API Dashboard (8 panels)

| Panel | What it shows |
|-------|---------------|
| Request Rate (req/s) | Throughput over time |
| Error Rate (%) | 5xx / total ratio |
| Request Latency (p50/p95/p99) | Latency distribution |
| Latency by Route (p95) | Hotspot identification |
| Active Requests | Concurrent in-flight requests |
| Request Rate (instant) | Current throughput gauge |
| P99 Latency (instant) | Current tail latency gauge |
| Error Rate (instant) | Current error percentage gauge |

### Training Dashboard (7 panels)

| Panel | What it shows |
|-------|---------------|
| Training Job Duration | Time per job (histogram) |
| Training Job Success Rate | Completed / total ratio |
| Active Training Jobs | Currently running jobs |
| Failed Jobs (last 1h) | Recent failure count |
| Completed Jobs (last 1h) | Recent completion count |
| GPU Memory Utilization | Memory usage during training |
| Training Throughput | samples/second across jobs |

### Temporal Dashboard

Temporal provides its own UI at `http://localhost:8088` showing:
- Workflow execution history
- Activity task status and retries
- Pending/running/completed/failed workflow counts
- Per-workflow execution timeline with input/output

---

## Alerting Rules

4 pre-configured Grafana alert rules in `infra/grafana/provisioning/alerting/rules.yml`:

| Alert | Condition | Severity | Duration |
|-------|-----------|----------|----------|
| **High API Error Rate** | 5xx rate > 5% | Critical | 5 min sustained |
| **High P99 Latency** | p99 > 2 seconds | Warning | 5 min sustained |
| **Temporal Queue Backup** | Queue depth > 500 | Warning | 10 min sustained |
| **Training Job Failures** | > 3 failures in 1 hour | Critical | Immediate |

Alerts can be routed to Slack, PagerDuty, email, or any Grafana-supported
notification channel via the Grafana alerting configuration.

---

## Infrastructure Components

| Component | Image | Port | Purpose |
|-----------|-------|------|---------|
| OTEL Collector | `otel/opentelemetry-collector-contrib` | 4317 (gRPC), 4318 (HTTP) | Receives and routes all telemetry |
| Prometheus | `prom/prometheus` | 9090 | Metrics storage + querying (30d retention) |
| Tempo | `grafana/tempo` | 3200 | Distributed trace storage |
| Loki | `grafana/loki` | 3100 | Log aggregation + querying |
| Grafana | `grafana/grafana` | 3001 | Visualization, dashboards, alerting |

All run on the shared `platform-net` Docker network with health checks
and resource limits (1 CPU / 512MB-1GB per service).

**OTEL Collector pipeline** (`infra/otel/otel-collector.yaml`):
```yaml
receivers:  [otlp]              # gRPC + HTTP
processors: [memory_limiter, batch]  # 512MB cap, 5s/1024 batch
exporters:
  traces  → otlp/tempo          # Tempo for trace storage
  metrics → prometheus           # Prometheus scrape endpoint
  logs    → loki                 # Loki push API
```

---

## What Each Component Observes

### Document Parsing (IngestWorkflow)
- **Logs**: document_id, mime_type, parse_quality, page_count, language, errors
- **Traces**: per-document span with S3 download + parse + upload
- **Temporal UI**: workflow execution, per-activity retries, heartbeat progress

### Data Generation (RefineWorkflow)
- **Logs**: chunk_count, pair_count, LLM API call success/failure per chunk
- **Traces**: workflow span with child spans for chunk/generate/build activities
- **Metrics**: LLM API circuit breaker state (open/closed/half-open)

### Training (TrainWorkflow)
- **Logs**: model loading, adapter attachment, per-phase completion
- **Real-time metrics** (Redis Stream → WebSocket/SSE):
  - step, total_steps, epoch
  - loss, learning_rate, grad_norm
  - eta_seconds, steps_per_second
  - gpu_utilization, gpu_memory_pct, gpu_temperature_c
- **Grafana**: job duration, success rate, GPU utilization, throughput
- **Temporal UI**: workflow timeline, activity heartbeats (`step=N/M`)

### Evaluation (EvaluateWorkflow)
- **Logs**: per-suite scores (domain, general, A/B, safety), recommendations
- **Traces**: evaluation span with child spans per suite
- **DB**: scores stored in `evaluations.scores` JSONB + `models.eval_scores`

### Inference (/v1/chat/completions)
- **Logs**: model_id, token usage, streaming vs buffered
- **Metrics**: request duration, token counts (via billing batcher)
- **Circuit breaker**: vLLM health tracking with automatic failover

### Deployment (DeploymentService)
- **Logs**: adapter load/unload, vLLM API responses, stale slot reaping
- **Metrics**: active adapter count per base model
- **Alerts**: deployment failures tracked via error rate alert

---

## Correlation: Connecting Logs, Traces, and Metrics

When `OTEL_ENABLED=true`, all three pillars are correlated:

1. **trace_id** is injected into every log line (via `LoggingInstrumentor` in Python,
   `tracing-opentelemetry` in Rust)
2. Grafana can jump from a log line → the trace that produced it → the metrics
   dashboard for that time window
3. `X-Request-Id` header provides a second correlation key for API requests

Example log line (JSON):
```json
{
  "timestamp": "2026-04-03T10:15:30.123Z",
  "level": "INFO",
  "logger": "platform.training",
  "message": "Training completed for job abc12345",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
  "span_id": "00f067aa0ba902b7",
  "request_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

In Grafana: click trace_id → see full distributed trace across API + Temporal + Worker.

---

## Quick Reference

```bash
# Start everything
make infra && make temporal && make observability

# Access points
Grafana:     http://localhost:3001  (admin/admin)
Prometheus:  http://localhost:9090
Tempo:       http://localhost:3200
Loki:        http://localhost:3100
Temporal UI: http://localhost:8088

# Enable in services
echo "OTEL_ENABLED=true" >> .env
echo "APP_OTEL_ENABLED=true" >> apps/workers/.env

# Shut down
make infra-down  # stops everything including observability
```
