#!/usr/bin/env bash
# Restores a dump produced by backup-db.sh. Destructive: the dump drops and
# recreates every object it owns. Pass a local path or an object key.
set -euo pipefail

cd "$(dirname "$0")"
[ $# -eq 1 ] || { echo "usage: $0 <local-dump.sql.gz | backups/postgres/NAME.sql.gz>"; exit 1; }
[ -f .env ] || { echo "restore-db: .env missing (run render-env.sh first)"; exit 1; }

env_get() { sed -n "s/^$1=//p" .env | head -1; }
APP_DB_PASSWORD=$(env_get APP_DB_PASSWORD)
S3_ENDPOINT=$(env_get S3_ENDPOINT)
S3_ACCESS_KEY=$(env_get S3_ACCESS_KEY)
S3_SECRET_KEY=$(env_get S3_SECRET_KEY)
S3_BUCKET=$(env_get S3_BUCKET)
S3_REGION=$(env_get S3_REGION)
[ -n "$APP_DB_PASSWORD" ] || { echo "restore-db: APP_DB_PASSWORD missing from .env"; exit 1; }

SRC=$1
if [ ! -f "$SRC" ]; then
    TMP=$(mktemp /tmp/restore.XXXXXX.sql.gz)
    docker run --rm \
        -e AWS_ACCESS_KEY_ID="$S3_ACCESS_KEY" \
        -e AWS_SECRET_ACCESS_KEY="$S3_SECRET_KEY" \
        -e AWS_DEFAULT_REGION="${S3_REGION:-auto}" \
        -v /tmp:/out \
        amazon/aws-cli:latest \
        s3 cp "s3://$S3_BUCKET/$SRC" "/out/$(basename "$TMP")" \
            --endpoint-url "$S3_ENDPOINT" --only-show-errors
    SRC=$TMP
fi

read -r -p "Restore $SRC over the live database? [y/N] " ok
[ "$ok" = "y" ] || { echo "aborted"; exit 1; }

gzip -dc "$SRC" | docker compose exec -T -e PGPASSWORD="$APP_DB_PASSWORD" postgres \
    psql -v ON_ERROR_STOP=1 -U platform -d platform
echo "restore-db: done"
