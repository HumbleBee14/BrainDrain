# Deploy box (OCI)

Terraform + docker-compose for the single production box. Configuration lives
in OCI Vault, and Redis/object storage stay managed (Upstash/R2).

**The box holds state.** Postgres runs here, so its data volume
(`app_postgres_data`) is the one thing a rebuild would destroy. Recreating the
instance costs whatever has not been backed up — see
[Backups](#backups) below. Everything else on the box is still disposable.

## Topology

- **Instance:** Always-Free ARM (`VM.Standard.A1.Flex`), Ubuntu 22.04,
  provisioned by Terraform in this directory. A **reserved public IP** is
  attached separately so the address (and DNS) survives instance recreation.
- **Services on the box:** Postgres (app + Temporal), API, workers, Temporal
  via [docker-compose.yml](docker-compose.yml), TLS terminated by Caddy.
  Postgres is self-hosted because metered managed tiers bill for compute the
  app's background loops hold awake continuously.
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


## Backups

Postgres lives on the box, so backups are not optional.

[backup-db.sh](backup-db.sh) dumps the app database, gzips it, and uploads to
`s3://$S3_BUCKET/backups/postgres/`. It refuses to upload an empty dump and
prunes local copies older than `BACKUP_RETAIN_DAYS` (default 14).

Install the nightly job once per box:

```bash
(crontab -l 2>/dev/null; echo "17 3 * * * ~/ekcron/infra/deploy/oracle/backup-db.sh >> ~/db-backup.log 2>&1") | crontab -
```

Restore, from a local file or an object key:

```bash
./restore-db.sh backups/postgres/platform-20260901T031700Z.sql.gz
```

Verify a restore actually works before you need it — an untested backup is a
guess.

## Required vault variables

Postgres credentials are the only database configuration; the connection URLs
are built in compose so they cannot drift from the running container.

| Variable | Purpose |
|---|---|
| `APP_DB_PASSWORD` | `platform` superuser — migrations and the app's main pool |
| `APP_RLS_PASSWORD` | `app_rls` least-privilege role used for tenant traffic |

`APP_RLS_PASSWORD` is applied by [initdb/01-app-rls-role.sh](initdb/01-app-rls-role.sh)
when the data volume is first created, so the role never keeps the dev password
that migration 017 falls back to.
