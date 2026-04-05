# PITR Backup and Restore

**PR:** #24  
**Problem:** A regular database dump (`pg_dump`) gives you a snapshot from when
the dump ran. If your database breaks at 2pm but the dump ran at 3am, you lose
11 hours of data. For a billing platform, that's unacceptable.

## What is PITR?

**Point-in-Time Recovery** lets you restore your database to **any specific
moment** — not just when a backup was taken. You could say "restore to
2026-04-05 at 14:29:00" and get exactly the database state from that second.

## How it works

PostgreSQL writes every change to a **WAL (Write-Ahead Log)** before applying
it. Normally these WAL files are recycled after use. With PITR, we **archive**
them to S3 storage continuously:

```
PostgreSQL writes → WAL files → wal-g archives to S3 (every 60 seconds)
                                     ↓
               Combined with periodic base backups (daily at 3am)
                                     ↓
               Can restore to any point between oldest backup and latest WAL
```

### The two pieces:

1. **Base backup** — A full snapshot of the database. Taken daily. This is
   the starting point for recovery.

2. **WAL archive** — Every change since the base backup. Streamed to S3
   continuously. This is the "replay log" that brings the base backup
   forward to any point in time.

## When would you use this?

- **Bad migration:** Someone ran a migration that corrupted data. Restore to
  1 minute before the migration ran.
- **Operator mistake:** Someone accidentally deleted rows. Restore to before
  the delete.
- **Data corruption:** Disk failure or bug caused data inconsistency. Restore
  to the last known-good state.

## Setup

```bash
# 1. Enable WAL archiving (one-time, requires Postgres restart)
WAL_S3_BUCKET=my-wal-bucket ./infra/pitr/enable-wal-archiving.sh

# 2. Schedule daily base backups (add to cron)
0 3 * * * /opt/platform/infra/pitr/backup.sh

# 3. Verify it's working
./infra/pitr/backup.sh --verify-only
```

## Recovery (the scary part, made safe)

```bash
# 1. Stop PostgreSQL
docker compose stop postgres

# 2. Restore to a specific time
./infra/pitr/restore.sh "2026-04-05 14:29:00+00"

# 3. Start PostgreSQL — it replays WAL to that time
docker compose start postgres

# 4. Verify, then re-enable archiving + take fresh backup
```

The restore script has safety checks: it won't run if Postgres is still
running, it backs up the existing data directory, and it prompts for
confirmation.

## Files

- `infra/pitr/enable-wal-archiving.sh` — Configure wal-g + S3 archiving
- `infra/pitr/backup.sh` — Full backup + retention (keeps last 7)
- `infra/pitr/restore.sh` — Point-in-time restore with safety checks
- `docs/PRODUCTION_OPS.md` — Full reference
