#!/bin/bash
# Enable WAL archiving on a PostgreSQL 16 instance.
#
# Prerequisites:
#   - S3-compatible storage (AWS S3, MinIO, R2) with a dedicated bucket
#   - wal-g installed on the Postgres host/container
#   - Superuser access to PostgreSQL
#
# This script configures PostgreSQL to continuously archive WAL segments
# to S3, enabling point-in-time recovery (PITR) to any moment after
# archiving starts.
#
# Usage:
#   WAL_S3_BUCKET=my-wal-bucket WAL_S3_ENDPOINT=https://s3.amazonaws.com \
#     ./enable-wal-archiving.sh

set -euo pipefail

# ── Configuration ──

WAL_S3_BUCKET="${WAL_S3_BUCKET:?Set WAL_S3_BUCKET}"
WAL_S3_ENDPOINT="${WAL_S3_ENDPOINT:-https://s3.amazonaws.com}"
WAL_S3_REGION="${WAL_S3_REGION:-us-east-1}"
WAL_S3_PREFIX="${WAL_S3_PREFIX:-wal-archive}"
PGDATA="${PGDATA:-/var/lib/postgresql/data}"

echo "=== Enabling WAL Archiving ==="
echo "  Bucket:   ${WAL_S3_BUCKET}"
echo "  Endpoint: ${WAL_S3_ENDPOINT}"
echo "  Region:   ${WAL_S3_REGION}"
echo "  Prefix:   ${WAL_S3_PREFIX}"
echo ""

# ── Verify prerequisites ──

if ! command -v wal-g &>/dev/null; then
    echo "ERROR: wal-g is not installed. Install it first:"
    echo "  https://github.com/wal-g/wal-g/releases"
    exit 1
fi

if ! pg_isready &>/dev/null; then
    echo "ERROR: PostgreSQL is not running or not reachable."
    exit 1
fi

# ── Configure wal-g (envdir layout) ──
# envdir expects a directory where each file's name is a variable name
# and its content is the value. This is what archive_command and
# restore_command use to load wal-g credentials.

WAL_G_DIR="/etc/wal-g/env"
mkdir -p "$WAL_G_DIR"

printf '%s' "s3://${WAL_S3_BUCKET}/${WAL_S3_PREFIX}" > "$WAL_G_DIR/WALG_S3_PREFIX"
printf '%s' "${WAL_S3_ENDPOINT}" > "$WAL_G_DIR/AWS_ENDPOINT"
printf '%s' "${WAL_S3_REGION}" > "$WAL_G_DIR/AWS_REGION"
printf '%s' "true" > "$WAL_G_DIR/AWS_S3_FORCE_PATH_STYLE"
printf '%s' "${PGDATA}" > "$WAL_G_DIR/PGDATA"

if [ -n "${AWS_ACCESS_KEY_ID:-}" ]; then
    printf '%s' "${AWS_ACCESS_KEY_ID}" > "$WAL_G_DIR/AWS_ACCESS_KEY_ID"
fi
if [ -n "${AWS_SECRET_ACCESS_KEY:-}" ]; then
    printf '%s' "${AWS_SECRET_ACCESS_KEY}" > "$WAL_G_DIR/AWS_SECRET_ACCESS_KEY"
fi

chmod 700 "$WAL_G_DIR"
chmod 600 "$WAL_G_DIR"/*
echo "Wrote wal-g envdir config to ${WAL_G_DIR}"

# ── Set PostgreSQL parameters ──

psql -U platform -d platform -c "
    ALTER SYSTEM SET wal_level = 'replica';
    ALTER SYSTEM SET archive_mode = 'on';
    ALTER SYSTEM SET archive_command = 'envdir /etc/wal-g/env wal-g wal-push %p';
    ALTER SYSTEM SET archive_timeout = '60';
"
echo "PostgreSQL archive settings applied."
echo ""
echo "IMPORTANT: PostgreSQL must be RESTARTED for wal_level and archive_mode"
echo "to take effect. These are startup-only parameters."
echo ""
echo "After restart, verify with:"
echo "  psql -c \"SHOW wal_level;\"          -- should be 'replica'"
echo "  psql -c \"SHOW archive_mode;\"       -- should be 'on'"
echo "  psql -c \"SELECT * FROM pg_stat_archiver;\"  -- check last_archived_wal"
