#!/bin/bash
# Pre-deployment verification checklist.
#
# Run this BEFORE deploying a new version to production.
# It verifies that the target environment is healthy and ready
# to accept a new deployment.
#
# Usage:
#   DATABASE_URL=postgres://... ./pre-deploy-check.sh
#   # optional:
#   # PGBOUNCER_HOST=pgbouncer PGBOUNCER_PORT=6432 ./pre-deploy-check.sh

set -eo pipefail

if [ -z "${DATABASE_URL:-}" ]; then
    echo "Usage: DATABASE_URL=postgres://... $0"
    echo "  DATABASE_URL is required to connect to the target database."
    exit 1
fi

PGBOUNCER_HOST="${PGBOUNCER_HOST:-}"
PGBOUNCER_PORT="${PGBOUNCER_PORT:-6432}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

PASS=0
FAIL=0
WARN=0

check_pass() { echo -e "  ${GREEN}✓${NC} $1"; PASS=$((PASS + 1)); }
check_fail() { echo -e "  ${RED}✗${NC} $1"; FAIL=$((FAIL + 1)); }
check_warn() { echo -e "  ${YELLOW}!${NC} $1"; WARN=$((WARN + 1)); }

echo "=== Pre-Deploy Verification ==="
echo ""

# ── 1. Database connectivity ──
echo "Database:"
if psql "${DATABASE_URL}" -c "SELECT 1" &>/dev/null; then
    check_pass "PostgreSQL is reachable"
else
    check_fail "Cannot connect to PostgreSQL"
fi

# Check migration state
FAILED_MIGRATIONS=$(psql "${DATABASE_URL}" -tAc \
    "SELECT COUNT(*) FROM _sqlx_migrations WHERE success = false" 2>/dev/null || echo "-1")
if [ "$FAILED_MIGRATIONS" = "0" ]; then
    check_pass "No failed migrations"
elif [ "$FAILED_MIGRATIONS" = "-1" ]; then
    check_warn "Cannot check migration state (new database?)"
else
    check_fail "${FAILED_MIGRATIONS} failed migration(s) — fix before deploying"
fi

# Check billing partitions
PARTITION_COUNT=$(psql "${DATABASE_URL}" -tAc \
    "SELECT COUNT(*) FROM pg_catalog.pg_inherits
     WHERE inhparent = 'billing_events'::regclass" 2>/dev/null || echo "0")
if [ "$PARTITION_COUNT" -gt 0 ]; then
    check_pass "Billing partitions exist (${PARTITION_COUNT} partitions)"
else
    check_warn "No billing partitions found (will be created by migration service)"
fi

# ── 2. WAL archiving (if configured) ──
echo ""
echo "Backup:"
WAL_LEVEL=$(psql "${DATABASE_URL}" -tAc "SHOW wal_level" 2>/dev/null || echo "unknown")
ARCHIVE_MODE=$(psql "${DATABASE_URL}" -tAc "SHOW archive_mode" 2>/dev/null || echo "unknown")
if [ "$WAL_LEVEL" = "replica" ] || [ "$WAL_LEVEL" = "logical" ]; then
    check_pass "wal_level is '${WAL_LEVEL}'"
else
    check_warn "wal_level is '${WAL_LEVEL}' — PITR not available"
fi

if [ "$ARCHIVE_MODE" = "on" ]; then
    check_pass "archive_mode is on"
    # Check for recent archive failures
    ARCHIVE_FAILS=$(psql "${DATABASE_URL}" -tAc \
        "SELECT failed_count FROM pg_stat_archiver" 2>/dev/null || echo "0")
    if [ "$ARCHIVE_FAILS" = "0" ]; then
        check_pass "No WAL archive failures"
    else
        check_warn "${ARCHIVE_FAILS} WAL archive failure(s) recorded"
    fi
else
    check_warn "archive_mode is '${ARCHIVE_MODE}' — WAL archiving not active"
fi

# ── 3. PgBouncer (if present) ──
echo ""
echo "Connection pooling:"
if [ -z "${PGBOUNCER_HOST}" ]; then
    check_warn "PGBOUNCER_HOST not set — skipping direct PgBouncer reachability check"
elif pg_isready -h "${PGBOUNCER_HOST}" -p "${PGBOUNCER_PORT}" &>/dev/null; then
    check_pass "PgBouncer is reachable"
else
    check_warn "PgBouncer not reachable at ${PGBOUNCER_HOST}:${PGBOUNCER_PORT}"
fi

# ── 4. Pending outbox rows ──
echo ""
echo "Billing outbox:"
PENDING_OUTBOX=$(psql "${DATABASE_URL}" -tAc \
    "SELECT COUNT(*) FROM billing_outbox WHERE delivered_at IS NULL" 2>/dev/null || echo "-1")
if [ "$PENDING_OUTBOX" = "-1" ]; then
    check_warn "Cannot check outbox (table may not exist yet)"
elif [ "$PENDING_OUTBOX" -lt 100 ]; then
    check_pass "Outbox pending: ${PENDING_OUTBOX} rows"
else
    check_warn "Outbox has ${PENDING_OUTBOX} pending rows — relay may be behind"
fi

STUCK_OUTBOX=$(psql "${DATABASE_URL}" -tAc \
    "SELECT COUNT(*) FROM billing_outbox WHERE attempt_count >= 5 AND delivered_at IS NULL" 2>/dev/null || echo "0")
if [ "$STUCK_OUTBOX" != "0" ] && [ "$STUCK_OUTBOX" != "-1" ]; then
    check_warn "${STUCK_OUTBOX} stuck outbox row(s) (attempt_count >= 5)"
fi

# ── Summary ──
echo ""
echo "=== Results ==="
echo -e "  ${GREEN}${PASS} passed${NC}  ${YELLOW}${WARN} warnings${NC}  ${RED}${FAIL} failed${NC}"
echo ""

if [ "$FAIL" -gt 0 ]; then
    echo -e "${RED}DEPLOY BLOCKED: Fix failures before proceeding.${NC}"
    exit 1
elif [ "$WARN" -gt 0 ]; then
    echo -e "${YELLOW}DEPLOY OK with warnings. Review before proceeding.${NC}"
    exit 0
else
    echo -e "${GREEN}All checks passed. Safe to deploy.${NC}"
    exit 0
fi
