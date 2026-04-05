# PgBouncer Connection Pooling

**PR:** #24  
**Problem:** Every API replica, worker, and background task opens its own pool
of PostgreSQL connections. With 5 API replicas × 20 connections each, that's
100 real Postgres connections — and Postgres gets slow and memory-hungry past
~200 connections.

## How it works

PgBouncer sits between your app and Postgres. Your app connects to PgBouncer
(port 6432), and PgBouncer maintains a much smaller pool of real connections
to Postgres (port 5432).

```
API (20 conns) ─┐
Workers (10)  ──┼── PgBouncer (6432) ──── PostgreSQL (5432)
Relay (5)     ──┘   20 real connections
```

200 client connections → 20 real Postgres connections. The multiplexing
happens at the transaction boundary: when your transaction commits, PgBouncer
gives that server connection to the next waiting client.

## Why transaction mode?

PgBouncer has three modes:
- **Session mode:** One server connection per client session. No multiplexing.
  Basically useless.
- **Transaction mode:** Server connection released after each transaction.
  This is what we use. Compatible with `SET LOCAL` and advisory locks within
  transactions.
- **Statement mode:** Server connection released after each statement. Breaks
  multi-statement transactions. Don't use.

Our platform uses `SET LOCAL` to set tenant context per transaction, which
auto-reverts when the transaction ends. This is fully compatible with
transaction mode.

## What bypasses PgBouncer

**Migrations** connect directly to Postgres because DDL statements (`CREATE
TABLE`, `ALTER TABLE`) and migration advisory locks need session-level state
that transaction-mode PgBouncer can't guarantee.

## Configuration

Override in `.env` or `docker-compose.prod.yml`:

| Variable | Default | What it controls |
|---|---|---|
| `PGBOUNCER_DEFAULT_POOL_SIZE` | 20 | Real Postgres connections |
| `PGBOUNCER_MAX_CLIENT_CONN` | 200 | Max client connections accepted |
| `PGBOUNCER_MIN_POOL_SIZE` | 5 | Keep-warm connections |
| `PGBOUNCER_RESERVE_POOL_SIZE` | 5 | Extra for burst traffic |

## Files

- `infra/pgbouncer/pgbouncer.ini` — Configuration
- `infra/pgbouncer/entrypoint.sh` — Generates auth file from env vars
- `docker-compose.prod.yml` — Service definition
- `docs/PRODUCTION_OPS.md` — Monitoring guide
