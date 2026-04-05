#!/bin/bash
# Point-in-time recovery (PITR) procedure.
#
# Restores the PostgreSQL cluster to a specific point in time using
# wal-g base backups + WAL archive replay.
#
# WARNING: This is a destructive operation. The existing database
# cluster will be replaced. Make sure you have a reason to do this.
#
# Prerequisites:
#   - PostgreSQL must be STOPPED before running this script
#   - wal-g configured with S3 access (same bucket as archiving)
#   - Target timestamp must be within the available WAL range
#
# Usage:
#   # Stop PostgreSQL first!
#   systemctl stop postgresql
#
#   # Restore to a specific point in time
#   ./restore.sh "2026-04-05 14:30:00+00"
#
#   # Restore to latest available (most recent WAL)
#   ./restore.sh latest
#
#   # Then start PostgreSQL
#   systemctl start postgresql

set -euo pipefail

TARGET="${1:-}"
PGDATA="${PGDATA:-/var/lib/postgresql/data}"

find_python() {
    if command -v python3 >/dev/null 2>&1; then
        echo "python3"
        return 0
    fi
    if command -v python >/dev/null 2>&1; then
        echo "python"
        return 0
    fi
    return 1
}

select_base_backup() {
    local target_ts="$1"
    local python_bin

    python_bin="$(find_python)" || {
        echo "ERROR: python3/python is required to select the correct base backup for PITR."
        echo "Install Python, or restore with TARGET=latest."
        exit 1
    }

    WAL_TARGET="$target_ts" wal-g backup-list --detail --json | "$python_bin" -c '
import json
import os
import sys
from datetime import UTC, datetime


def parse_dt(value, fmt=None):
    if not value:
        return None
    if fmt:
        try:
            dt = datetime.strptime(value, fmt)
            return dt.replace(tzinfo=UTC) if dt.tzinfo is None else dt.astimezone(UTC)
        except ValueError:
            pass
    normalized = value.replace("Z", "+00:00")
    try:
        dt = datetime.fromisoformat(normalized)
        return dt.replace(tzinfo=UTC) if dt.tzinfo is None else dt.astimezone(UTC)
    except ValueError:
        return None


def backup_name(item):
    return item.get("backup_name") or item.get("backupName") or item.get("BackupName")


def backup_time(item):
    return item.get("time") or item.get("backup_time") or item.get("start_time") or item.get("modified")


def as_list(payload):
    if isinstance(payload, list):
        return payload
    if isinstance(payload, dict):
        for key in ("backups", "Backups"):
            if key in payload and isinstance(payload[key], list):
                return payload[key]
    raise SystemExit("ERROR: Unexpected wal-g backup-list JSON format")


target_raw = os.environ["WAL_TARGET"]
target_dt = parse_dt(target_raw)
if target_dt is None:
    raise SystemExit(f"ERROR: Could not parse target timestamp: {target_raw}")

payload = json.load(sys.stdin)
backups = []
for item in as_list(payload):
    name = backup_name(item)
    ts = parse_dt(backup_time(item), item.get("date_fmt"))
    if name and ts:
        backups.append((ts, name))

if not backups:
    raise SystemExit("ERROR: No parseable backups found in wal-g backup-list output")

eligible = [entry for entry in backups if entry[0] <= target_dt]
if not eligible:
    raise SystemExit("ERROR: No base backup exists at or before the requested recovery target")

eligible.sort(key=lambda entry: entry[0])
print(eligible[-1][1])
'
}

if [ -z "$TARGET" ]; then
    echo "Usage: $0 <target-timestamp|latest>"
    echo ""
    echo "Examples:"
    echo "  $0 '2026-04-05 14:30:00+00'    # Restore to specific time"
    echo "  $0 latest                        # Restore to latest WAL"
    echo ""
    echo "IMPORTANT: Stop PostgreSQL before running this script."
    exit 1
fi

# Load wal-g environment from envdir layout.
if [ -d /etc/wal-g/env ]; then
    for f in /etc/wal-g/env/*; do
        [ -f "$f" ] || continue
        export "$(basename "$f")=$(cat "$f")"
    done
fi

# ── Safety checks ──

if pg_isready &>/dev/null; then
    echo "ERROR: PostgreSQL is still running. Stop it first:"
    echo "  systemctl stop postgresql"
    echo "  # or: docker compose stop postgres"
    exit 1
fi

if [ ! -d "$PGDATA" ]; then
    echo "ERROR: PGDATA directory not found: $PGDATA"
    exit 1
fi

echo "=== Point-in-Time Recovery ==="
echo "  Target: ${TARGET}"
echo "  PGDATA: ${PGDATA}"
echo ""

# ── List available backups ──

echo "Available backups:"
wal-g backup-list --detail --pretty
echo ""

# ── Confirm ──

echo "WARNING: This will REPLACE the current database cluster at ${PGDATA}."
echo "The existing data directory will be moved to ${PGDATA}.old.<timestamp>"
read -p "Type 'yes' to continue: " CONFIRM
if [ "$CONFIRM" != "yes" ]; then
    echo "Aborted."
    exit 1
fi

# ── Move existing data directory ──
# Compute timestamp right before the move so it matches the actual operation.

BACKUP_DIR="${PGDATA}.old.$(date +%Y%m%d%H%M%S)"
echo "Moving existing PGDATA to ${BACKUP_DIR}..."
mv "$PGDATA" "$BACKUP_DIR"

# ── Restore base backup ──

if [ "$TARGET" = "latest" ]; then
    BACKUP_NAME="LATEST"
else
    BACKUP_NAME="$(select_base_backup "$TARGET")"
fi

echo "Restoring base backup: ${BACKUP_NAME}"
wal-g backup-fetch "$PGDATA" "$BACKUP_NAME"

# ── Configure recovery target ──

# Strip any prior PITR recovery lines from postgresql.auto.conf (if it exists),
# then append the new recovery settings. This preserves ALTER SYSTEM settings.
AUTO_CONF="${PGDATA}/postgresql.auto.conf"
if [ -f "$AUTO_CONF" ]; then
    grep -v -E "^(# PITR recovery|restore_command|recovery_target|recovery_target_time|recovery_target_action)" \
        "$AUTO_CONF" > "${AUTO_CONF}.tmp" || true
    mv "${AUTO_CONF}.tmp" "$AUTO_CONF"
fi

if [ "$TARGET" = "latest" ]; then
    echo "Configuring recovery to latest available WAL..."
    # No recovery_target — PostgreSQL replays all available WAL then promotes.
    cat >> "$AUTO_CONF" <<EOF
# PITR recovery configuration — auto-generated by restore.sh
restore_command = '/usr/local/bin/wal-g-wrapper wal-fetch %f %p'
recovery_target_timeline = 'latest'
recovery_target_action = 'promote'
EOF
else
    echo "Configuring recovery to target time: ${TARGET}..."
    cat >> "$AUTO_CONF" <<EOF
# PITR recovery configuration — auto-generated by restore.sh
restore_command = '/usr/local/bin/wal-g-wrapper wal-fetch %f %p'
recovery_target_time = '${TARGET}'
recovery_target_action = 'promote'
EOF
fi

# Signal PostgreSQL to enter recovery mode
touch "${PGDATA}/recovery.signal"

echo ""
echo "=== Restore prepared ==="
echo ""
echo "Next steps:"
echo "  1. Start PostgreSQL:"
echo "     systemctl start postgresql"
echo "     # or: docker compose start postgres"
echo ""
echo "  2. PostgreSQL will replay WAL segments until the target time."
echo "     Monitor progress in the PostgreSQL log."
echo ""
echo "  3. Verify recovery:"
echo "     psql -c 'SELECT pg_is_in_recovery();'  -- should be 'f' after promote"
echo "     psql -c 'SELECT NOW();'                 -- sanity check"
echo ""
echo "  4. After verification, re-enable WAL archiving:"
echo "     ./enable-wal-archiving.sh"
echo ""
echo "  5. Take a fresh base backup:"
echo "     ./backup.sh"
echo ""
echo "  6. The old data directory is at: ${BACKUP_DIR}"
echo "     Remove it after confirming the restore is good:"
echo "     rm -rf ${BACKUP_DIR}"
