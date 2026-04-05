#!/bin/sh
set -e

# Set defaults for variables used in pgbouncer.ini template.
# envsubst only expands $VAR/${VAR}, not ${VAR:-default}.
: "${DB_HOST:=postgres}"
: "${DB_PORT:=5432}"
: "${DB_NAME:=platform}"
: "${DB_USER:=platform}"
: "${DB_PASSWORD:?DB_PASSWORD is required}"
: "${PGBOUNCER_ADMIN_PASSWORD:?PGBOUNCER_ADMIN_PASSWORD is required}"
: "${PGBOUNCER_DEFAULT_POOL_SIZE:=20}"
: "${PGBOUNCER_MAX_CLIENT_CONN:=200}"
: "${PGBOUNCER_MIN_POOL_SIZE:=5}"
: "${PGBOUNCER_RESERVE_POOL_SIZE:=5}"
export DB_HOST DB_PORT DB_NAME DB_USER DB_PASSWORD
export PGBOUNCER_ADMIN_PASSWORD
export PGBOUNCER_DEFAULT_POOL_SIZE PGBOUNCER_MAX_CLIENT_CONN
export PGBOUNCER_MIN_POOL_SIZE PGBOUNCER_RESERVE_POOL_SIZE

# Generate userlist.txt for client authentication.
echo "\"${DB_USER}\" \"${DB_PASSWORD}\"" > /etc/pgbouncer/userlist.txt
echo "\"pgbouncer_admin\" \"${PGBOUNCER_ADMIN_PASSWORD}\"" >> /etc/pgbouncer/userlist.txt
chmod 600 /etc/pgbouncer/userlist.txt

# Expand environment variables in the config template.
envsubst < /etc/pgbouncer/pgbouncer.ini > /tmp/pgbouncer.ini

exec pgbouncer /tmp/pgbouncer.ini
