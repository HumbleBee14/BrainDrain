#!/bin/bash
# Create a full base backup using wal-g.
#
# This creates a consistent snapshot of the entire PostgreSQL cluster
# and uploads it to S3. Combined with continuous WAL archiving, this
# enables point-in-time recovery to any moment between this backup
# and the latest archived WAL segment.
#
# Schedule this via cron (recommended: daily at low-traffic time):
#   0 3 * * * /path/to/backup.sh >> /var/log/platform/backup.log 2>&1
#
# Usage:
#   ./backup.sh                    # Full backup + verify archiver health
#   ./backup.sh --verify-only      # Just check archiver status

set -euo pipefail

# Load wal-g environment from envdir layout.
# Each file in /etc/wal-g/env/ is named after a variable, content is the value.
if [ -d /etc/wal-g/env ]; then
    for f in /etc/wal-g/env/*; do
        [ -f "$f" ] || continue
        export "$(basename "$f")=$(cat "$f")"
    done
fi

VERIFY_ONLY="${1:-}"

echo "=== Platform Backup $(date -Iseconds) ==="

# ── Verify archiver health ──

echo "Checking WAL archiver status..."
ARCHIVER_STATUS=$(psql -U platform -d platform -tAc "
    SELECT json_build_object(
        'archived_count', archived_count,
        'failed_count', failed_count,
        'last_archived_wal', last_archived_wal,
        'last_archived_time', last_archived_time,
        'last_failed_wal', last_failed_wal,
        'last_failed_time', last_failed_time
    ) FROM pg_stat_archiver;
")
echo "  Archiver: ${ARCHIVER_STATUS}"

# Check for recent failures
FAILED_COUNT=$(echo "$ARCHIVER_STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin)['failed_count'])" 2>/dev/null || echo "0")
if [ "$FAILED_COUNT" -gt 0 ]; then
    echo "WARNING: pg_stat_archiver shows ${FAILED_COUNT} failed WAL archives."
    echo "Check last_failed_wal and archive_command logs."
fi

# Verify wal_level
WAL_LEVEL=$(psql -U platform -d platform -tAc "SHOW wal_level;")
if [ "$WAL_LEVEL" != "replica" ] && [ "$WAL_LEVEL" != "logical" ]; then
    echo "ERROR: wal_level is '${WAL_LEVEL}', expected 'replica' or 'logical'."
    echo "WAL archiving is not properly configured. Run enable-wal-archiving.sh first."
    exit 1
fi

ARCHIVE_MODE=$(psql -U platform -d platform -tAc "SHOW archive_mode;")
if [ "$ARCHIVE_MODE" != "on" ]; then
    echo "ERROR: archive_mode is '${ARCHIVE_MODE}', expected 'on'."
    exit 1
fi

echo "  wal_level: ${WAL_LEVEL}"
echo "  archive_mode: ${ARCHIVE_MODE}"

if [ "$VERIFY_ONLY" = "--verify-only" ]; then
    echo "Verification complete."
    exit 0
fi

# ── Full base backup ──

echo "Starting full base backup..."
wal-g backup-push "${PGDATA:-/var/lib/postgresql/data}"
echo "Base backup completed."

# ── List recent backups ──

echo ""
echo "Recent backups:"
wal-g backup-list

# ── Retention policy: keep last 7 daily backups ──

echo ""
echo "Applying retention policy (keep last 7 backups)..."
wal-g delete retain FULL 7 --confirm
echo "Retention cleanup complete."

echo ""
echo "=== Backup finished $(date -Iseconds) ==="
