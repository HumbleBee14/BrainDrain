# Ekcron

**Turn a large, expensive model into a small, cheap one that's just as good at your task.**

Frontier models are excellent generalists and terrible economics. Most production workloads are narrow — one domain, one format, one set of tools — and pay generalist prices on every call. Ekcron closes that gap: it distills the behaviour you actually need into a small model you own, then serves it behind an OpenAI-compatible endpoint.

You bring documents or examples. The platform produces training data, fine-tunes a small model, measures it against the model it's replacing, and deploys it. No ML expertise required, no notebooks.

**Live:** [ekcron.com](https://ekcron.com) · dashboard at [app.ekcron.com](https://app.ekcron.com) · API at `api.ekcron.com`

## Why this shape

- **Small models win when the task is narrow.** A 1–8B model with a good adapter beats a giant general model on cost and latency, and often on quality.
- **Distillation needs a trust harness, not vibes.** Every run is scored against the base model on the same data, so "is the small one good enough?" is a number, not a guess.
- **Your keys, your data.** Data generation and evaluation call whichever LLM provider you configure, with your own key. The platform ships no default key.

## Pipeline

```
Upload  ->  Parse  ->  Generate  ->  Train  ->  Evaluate  ->  Deploy
 docs      text/      synthetic     LoRA /     tuned vs      OpenAI-compatible
           chunks     Q&A pairs     QLoRA      base score    inference
```

Every stage is a durable Temporal workflow: retryable, observable, and safe to resume after a crash.

## How it runs in production

Two independent planes, so idle cost stays near zero.

**Control plane — one small ARM VM (OCI), always on**

Rust API, Python Temporal workers, Temporal server, and Caddy for TLS, as containers via Docker Compose. It orchestrates and stores; it never touches a GPU. Postgres, Redis, and object storage are managed services.

**Compute plane — Modal, scale-to-zero**

GPUs are rented per job and released. Nothing runs between jobs.

| Job | Where | Notes |
|---|---|---|
| Fine-tuning (SFT/DPO/GRPO) | Modal GPU, per run | Unsloth/TRL; adapter is uploaded to object storage |
| Inference | Modal + vLLM, scale-to-zero | Boots on first request, sleeps after idle |
| GGUF export | Modal CPU | Merge + quantize needs no GPU |

The worker dispatches GPU work through a provider interface, so the same activity code runs against a local GPU or a cloud one.

**Serving detail worth knowing:** one vLLM instance serves many fine-tunes. Adapters are fetched lazily from object storage on first use and addressed by the `model` field, so a new fine-tune costs no new GPU — it joins the running instance.

```
Browser ─→ app.ekcron.com (Next.js, edge)
              │
              └─→ api.ekcron.com (Rust/Axum on OCI) ──→ Postgres · Redis · S3
                        │
                        ├── Temporal ──→ Python workers ──→ Modal GPU (train / eval)
                        └── inference proxy ───────────────→ Modal vLLM (+ LoRA adapters)
```

## Services

| Service | Stack | Responsibility |
|---|---|---|
| **API** | Rust (Axum, SQLx) | Auth, multi-tenancy, billing, inference routing, deployment control plane |
| **Workers** | Python (Temporal, Unsloth, TRL) | Parsing, data generation, training, evaluation, export |
| **Web** | Next.js 15 | Dashboard, live training metrics, playground |

Independently deployable, own containers, no runtime coupling. Rust API types are the single source of truth — TypeScript is generated from them.

## Roadmap

Distillation for agents is the direction: capture what a large model does on real traffic, including tool calls, and train a small model to do it for a fraction of the cost.

- On-policy distillation from live traffic
- Tool-call and agent-trajectory training
- Multi-adapter serving and adapter composition
- A parity harness that certifies "small model is safe to swap in"

## Local development

```bash
make infra          # PostgreSQL, Redis, MinIO
make temporal       # Temporal server
make migrate        # Run DB migrations
make dev-api        # Rust API on :8000
make dev-web        # Next.js on :3000
make dev-workers    # Python workers
```

Set your LLM provider key in the dashboard under **Settings → LLM** — data generation and evaluation both need it.

## Documentation

| Document | What it covers |
|---|---|
| [Project Flow](docs/PROJECT_FLOW.md) | End-to-end pipeline: upload to deployed model |
| [Architecture](docs/SYSTEM_ARCHITECTURE.md) | System design, control plane, infrastructure |
| [Development](docs/DEVELOPMENT.md) | Setup, conventions, testing |
