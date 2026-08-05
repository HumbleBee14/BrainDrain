# Deploy box (OCI)

Terraform + docker-compose for the single production box. The box is
**disposable**: all state lives in managed services (Postgres, Redis, object
storage) and all configuration lives in OCI Vault — destroying and recreating
the instance loses nothing.

## Topology

- **Instance:** Always-Free ARM (`VM.Standard.A1.Flex`), Ubuntu 22.04,
  provisioned by Terraform in this directory. A **reserved public IP** is
  attached separately so the address (and DNS) survives instance recreation.
- **Services on the box:** API + workers + Temporal via
  [docker-compose.yml](docker-compose.yml), TLS terminated by Caddy.
- **First boot:** [cloud-init.yaml](cloud-init.yaml) installs Docker and the
  OCI CLI. Nothing app-specific — app config comes from Vault at deploy time.

## Configuration: OCI Vault, not hand-edited files

The box's entire `.env` is stored as **one Vault secret** (`<name>-box-env`).
Nobody edits `.env` on the box; it is rendered from the vault on every deploy.

```text
operator laptop ──set-env-secret.sh──▶ OCI Vault secret (versioned)
                                             │
                                   render-env.sh (on box,
                                   instance-principal auth, read-only)
                                             │
                                          .env ──▶ docker compose
```

- [secrets.tf](secrets.tf) declares the vault, key, secret **container**, the
  box's dynamic group, and a read-only policy scoped to that single secret.
  Secret *values* never enter git or Terraform state.
- [get-env-secret.sh](get-env-secret.sh) — operator-side read of the current
  version (auth from `~/.oci/config`, same as Terraform).
- [set-env-secret.sh](set-env-secret.sh) — operator-side push of a new
  version. The box cannot write its own config.
- [render-env.sh](render-env.sh) — runs on the box during deploy; fetches the
  CURRENT secret version and writes `.env` atomically. Fails loud: on any
  error the existing `.env` is untouched and the deploy aborts.

### Add or change a secret / env variable

All commands run from this directory on the operator's machine:

```bash
# 1. Fetch the current env from the vault
./get-env-secret.sh > /tmp/box.env

# 2. Edit it — change a value or append a new variable
echo 'APP_MY_NEW_SECRET=value' >> /tmp/box.env   # or: $EDITOR /tmp/box.env

# 3. Push it back as a new secret version, then remove the local copy
./set-env-secret.sh /tmp/box.env
rm /tmp/box.env

# 4. Roll it out (compose restarts only containers whose env changed)
gh workflow run "Build and Deploy"
```

For a **brand-new** variable there is one extra step: compose only injects
variables it declares, so add a passthrough line for it in
[docker-compose.yml](docker-compose.yml) under the right service, e.g.
`APP_MY_NEW_SECRET: ${APP_MY_NEW_SECRET:-}`. Merging that to `main`
triggers the deploy, so steps 1–3 (push the value first) then the compose
PR is the natural order.

## Deploys

Pushes to `main` touching app or infra paths build arm64 images to GHCR and
deploy over SSH (`.github/workflows/deploy-staging.yml`): git pull →
`render-env.sh` → `docker compose pull` → migrate → `up -d`.

## Provision / recreate the box

```bash
terraform init
terraform plan        # review: should never replace the instance by surprise
terraform apply
```

The instance ignores drift in `user_data` (cloud-init runs only on first
boot) and in the floating "latest Ubuntu image" — so routine plans can never
schedule a replacement of the live box. Recreate deliberately:

```bash
terraform apply -replace=oci_core_instance.box
```

Bootstrap a fresh box (once, after apply):

```bash
# point render-env.sh at the vault secret
terraform output -raw env_secret_ocid | \
  ssh -i ~/.ssh/ekcron_box ubuntu@<reserved-ip> \
  'mkdir -p ~/ekcron && cat > ~/ekcron/infra/deploy/oracle/.secret-ocid'
# clone the repo, then let the deploy workflow do the rest
```

## Auth

- Terraform + `set-env-secret.sh`: `~/.oci/config` `[DEFAULT]` profile.
- The box: instance-principal (no credentials stored on it).
- Cloudflare DNS record: API token in `terraform.tfvars` (gitignored).
