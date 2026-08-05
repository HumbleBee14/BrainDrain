#!/usr/bin/env bash
# Renders .env from the OCI Vault secret using instance-principal auth.
# Runs ON THE BOX before `docker compose up`. Fails loud: on any error the
# existing .env is left untouched and the deploy aborts.
#
# Bootstrap (once per box): write the secret OCID to .secret-ocid, e.g.
#   terraform output -raw env_secret_ocid | ssh box 'cat > ~/ekcron/infra/deploy/oracle/.secret-ocid'
set -euo pipefail

cd "$(dirname "$0")"

OCID_FILE=".secret-ocid"
[ -f "$OCID_FILE" ] || { echo "render-env: missing $OCID_FILE (see header)"; exit 1; }
SECRET_OCID=$(tr -d '[:space:]' < "$OCID_FILE")

OCI_BIN=$(command -v oci || echo "$HOME/.local/bin/oci")
[ -x "$OCI_BIN" ] || { echo "render-env: oci CLI not installed"; exit 1; }

TMP=$(mktemp .env.render.XXXXXX)
trap 'rm -f "$TMP"' EXIT
chmod 600 "$TMP"

"$OCI_BIN" secrets secret-bundle get \
  --auth instance_principal \
  --secret-id "$SECRET_OCID" \
  --stage CURRENT \
  --query 'data."secret-bundle-content".content' \
  --raw-output | base64 -d > "$TMP"

[ -s "$TMP" ] || { echo "render-env: fetched secret is empty, aborting"; exit 1; }

mv "$TMP" .env
trap - EXIT
echo "render-env: .env rendered ($(grep -c '=' .env) variables)"
