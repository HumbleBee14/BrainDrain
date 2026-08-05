#!/usr/bin/env bash
# Prints the CURRENT app-env secret (dotenv) to stdout.
# Runs on the OPERATOR's machine (auth from ~/.oci/config, same as Terraform).
#
# Usage: ./get-env-secret.sh > /tmp/box.env
set -euo pipefail

cd "$(dirname "$0")"

command -v oci >/dev/null || { echo "get-env-secret: oci CLI not installed (brew install oci-cli)" >&2; exit 1; }

SECRET_OCID=$(terraform output -raw env_secret_ocid)

oci secrets secret-bundle get \
  --secret-id "$SECRET_OCID" \
  --stage CURRENT \
  --query 'data."secret-bundle-content".content' \
  --raw-output | base64 -d
