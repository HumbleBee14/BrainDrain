#!/bin/bash
set -e

echo "Waiting for PostgreSQL to be ready..."
until pg_isready -h localhost -p 5432 -U platform; do
  sleep 1
done

echo "Running migrations..."
cargo run -p platform-db --bin migrate

echo "Database initialized successfully."
