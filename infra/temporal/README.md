# infra/temporal

**Temporal server for durable workflow orchestration.**

| | |
|---|---|
| Type | Docker Compose infrastructure |
| Components | Temporal server, PostgreSQL (Temporal's own DB), Temporal Web UI |
| Port (gRPC) | 7233 |
| Port (Web UI) | 8088 |

## What Is Temporal

Temporal is a workflow orchestration engine. It guarantees that your ML pipeline stages (parse → refine → train → evaluate → deploy) run to completion even if machines crash, processes restart, or GPUs fail mid-training. Each step is individually retryable with configurable timeouts.

## Why Separate Docker Compose

Temporal needs its own PostgreSQL instance (separate from the app DB). Keeping it in a separate compose file means you can start/stop it independently and it doesn't clutter the main `docker-compose.yml`.

## Running

```bash
docker compose -f infra/temporal/docker-compose.temporal.yml up -d
```

Web UI: `http://localhost:8088` — view running workflows, activity history, and retry failed tasks.
