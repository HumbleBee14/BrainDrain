# Platform

Project: An end-to-end LLM fine-tuning platform where users upload raw data, the system handles data curation and generates training data, and fine-tunes models, evaluates quality, and serves the result.

## Quick Start

```bash
make lint && make test
docker compose up -d              # Start PostgreSQL, Redis, MinIO
make migrate                      # Run database migrations
make dev-api                      # Start Rust API server
cd apps/web && pnpm install && pnpm dev  # Start frontend

```

## Documentation

| Document                                     | Description                                                         |
| -------------------------------------------- | ------------------------------------------------------------------- |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System architecture, technical decision records, component registry |
| [docs/RESEARCH.md](docs/RESEARCH.md)         | LLM fine-tuning landscape research and analysis                     |
