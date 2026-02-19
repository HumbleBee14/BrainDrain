# infra/scripts

**Infrastructure utility scripts for local development and CI.**

| Script | Purpose |
|---|---|
| `init-db.sh` | Creates the PostgreSQL database and runs initial setup. Used by Docker Compose health checks and CI. |

## Adding New Scripts

Place any infrastructure automation here — seed data, backup/restore, SSL cert generation, etc. Keep them small and single-purpose.
