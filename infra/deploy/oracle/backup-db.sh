#!/usr/bin/env bash
# Nightly logical backup of the app database to object storage.
# Runs ON THE BOX. Fails loud: a partial or empty dump is never uploaded.
set -euo pipefail

cd "$(dirname "$0")"
[ -f .env ] || { echo "backup-db: .env missing (run render-env.sh first)"; exit 1; }

# Read individual keys rather than sourcing: .env holds unquoted JSON values
# that the shell would try to execute.
env_get() { sed -n "s/^$1=//p" .env | head -1; }

APP_DB_PASSWORD=$(env_get APP_DB_PASSWORD)
S3_ENDPOINT=$(env_get S3_ENDPOINT)
S3_ACCESS_KEY=$(env_get S3_ACCESS_KEY)
S3_SECRET_KEY=$(env_get S3_SECRET_KEY)
S3_BUCKET=$(env_get S3_BUCKET)
S3_REGION=$(env_get S3_REGION)
RETAIN_DAYS=$(env_get BACKUP_RETAIN_DAYS)

for v in APP_DB_PASSWORD S3_ENDPOINT S3_ACCESS_KEY S3_SECRET_KEY S3_BUCKET; do
    eval "[ -n \"\${$v}\" ]" || { echo "backup-db: $v missing from .env"; exit 1; }
done
RETAIN_DAYS=${RETAIN_DAYS:-14}
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
OUT_DIR=${BACKUP_DIR:-$HOME/db-backups}
FILE="$OUT_DIR/platform-$STAMP.sql.gz"
mkdir -p "$OUT_DIR"

docker compose exec -T -e PGPASSWORD="$APP_DB_PASSWORD" postgres \
    pg_dump -U platform -d platform --clean --if-exists \
  | gzip -9 > "$FILE"

# gzip of an empty dump is still ~20 bytes, so check the real payload.
if [ "$(gzip -dc "$FILE" | head -c 100 | wc -c)" -lt 100 ]; then
    echo "backup-db: dump looks empty, refusing to upload"; rm -f "$FILE"; exit 1
fi

docker run --rm \
    -e AWS_ACCESS_KEY_ID="$S3_ACCESS_KEY" \
    -e AWS_SECRET_ACCESS_KEY="$S3_SECRET_KEY" \
    -e AWS_DEFAULT_REGION="${S3_REGION:-auto}" \
    -v "$OUT_DIR:/backups:ro" \
    amazon/aws-cli:latest \
    s3 cp "/backups/$(basename "$FILE")" \
        "s3://$S3_BUCKET/backups/postgres/$(basename "$FILE")" \
        --endpoint-url "$S3_ENDPOINT" --only-show-errors

find "$OUT_DIR" -name 'platform-*.sql.gz' -mtime "+$RETAIN_DAYS" -delete
echo "backup-db: uploaded $(basename "$FILE") ($(du -h "$FILE" | cut -f1))"
