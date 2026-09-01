#!/bin/sh
# Provisions the least-privilege RLS role before migration 017 runs, so the
# app never falls back to that migration's dev password. Runs once, on first
# initialisation of the data volume.
set -e

[ -n "$APP_RLS_PASSWORD" ] || { echo "initdb: APP_RLS_PASSWORD is required"; exit 1; }

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<SQL
DO \$\$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_rls') THEN
        CREATE ROLE app_rls LOGIN PASSWORD '$APP_RLS_PASSWORD';
    ELSE
        ALTER ROLE app_rls WITH LOGIN PASSWORD '$APP_RLS_PASSWORD';
    END IF;
END
\$\$;
SQL
