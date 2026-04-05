#!/bin/sh
set -e

# Generate userlist.txt from environment variables.
# PgBouncer uses this to authenticate incoming client connections.
# Format: "username" "md5<hash>" or "username" "password"
echo "\"${DB_USER:-platform}\" \"${DB_PASSWORD:?DB_PASSWORD is required}\"" > /etc/pgbouncer/userlist.txt

# Also add the admin user for the PgBouncer console
echo "\"pgbouncer_admin\" \"${PGBOUNCER_ADMIN_PASSWORD:-pgbouncer_admin}\"" >> /etc/pgbouncer/userlist.txt

chmod 600 /etc/pgbouncer/userlist.txt

# Substitute environment variables in pgbouncer.ini
# PgBouncer doesn't natively support env var expansion, so we do it here.
envsubst < /etc/pgbouncer/pgbouncer.ini > /tmp/pgbouncer.ini

exec pgbouncer /tmp/pgbouncer.ini
