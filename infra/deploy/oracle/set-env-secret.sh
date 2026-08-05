#!/usr/bin/env bash
# Pushes a dotenv file as a new version of the OCI Vault app-env secret.
# Runs on the OPERATOR's machine (auth from ~/.oci/config, same as Terraform).
# The box only ever reads; all writes go through this script.
#
# Usage: ./set-env-secret.sh <path-to-env-file>
# After pushing, trigger the deploy workflow (or push to main) to roll it out.
set -euo pipefail

cd "$(dirname "$0")"

ENV_FILE="${1:?usage: $0 <path-to-env-file>}"
[ -s "$ENV_FILE" ] || { echo "set-env-secret: $ENV_FILE is missing or empty"; exit 1; }

command -v oci >/dev/null || { echo "set-env-secret: oci CLI not installed (brew install oci-cli)"; exit 1; }

SECRET_OCID=$(terraform output -raw env_secret_ocid)

oci vault secret update-base64 \
  --secret-id "$SECRET_OCID" \
  --secret-content-content "$(base64 -i "$ENV_FILE")" \
  --query 'data."lifecycle-state"' \
  --raw-output

echo "set-env-secret: new version pushed ($(grep -c '=' "$ENV_FILE") variables). Deploy to apply."
