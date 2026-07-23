# Cloud GPU Training

How BrainDrain offloads model fine-tuning to serverless GPUs (Modal) instead
of requiring every worker host to carry an attached CUDA GPU, and how to
extend this to other GPU providers.

## 1. Overview

Training a model is normally the one step in the pipeline that needs a real
GPU. Most of the platform (parsing, data generation, dataset assembly) is
CPU-only and runs comfortably on any worker host. Cloud GPU training lets a
CPU-only worker dispatch the GPU-bound part of a training job to a
serverless GPU provider (Modal today) and get the result back, without the
worker itself ever touching CUDA.

End-to-end flow for a single-shot job (`quick` / `aligned` / `reasoning`
mode, driven by `TrainWorkflow` → `start_training` activity):

```
Temporal worker (CPU host, ml-pipeline-gpu queue)
  StartTrainingActivity.run()
    1. UPDATE training_jobs SET status = 'training' (DB)
    2. resolve tenant llm_config from DB
    3. gpu_provider.run_training(...)         <- ModalGpuProvider
         a. spawn deployed Modal function      (payload: input + llm_config)
         b. UPDATE training_jobs SET modal_call_id = ...   <- reservation write
         c. poll FunctionCall.get(timeout=0) in a loop, heartbeating
                                    │
                                    │  (network: S3 + judge LLM only)
                                    ▼
                         Modal GPU container (A10/T4/H100/...)
                           run_training_core(...)
                             - download dataset from S3
                             - load base model + attach LoRA/QLoRA adapter
                             - run the training strategy (SFT/DPO/GRPO)
                             - upload adapter to S3
                             - return {adapter_path, adapter_size_bytes, metrics}
                                    │
                                    ▼
    4. result comes back to the worker via the poll loop
    5. single DB transaction: training_jobs -> completed, insert models row,
       append billing_outbox row
```

The Modal container never talks to Postgres — it only reaches S3 (to move
the dataset and adapter) and the tenant's judge LLM endpoint (for DPO/GRPO
reward scoring). By default it also doesn't touch Redis: the remote entrypoint
`setdefault`s `APP_METRICS_BACKEND=log` before loading settings, so training
metrics go to the container's log output (visible via Modal logs / Temporal
heartbeat detail) and progress reaches the worker as coarse activity
heartbeats. If you want live per-step metrics, set `APP_METRICS_BACKEND=redis`
plus a **publicly reachable** `APP_REDIS_URL` (e.g. an Upstash `rediss://` URL —
Modal cannot reach a compose-internal `redis://localhost:6379`) in the Modal
secret; the remote then streams to that Redis instead. All state changes
(job status, model row, billing ledger) are
written by the worker process after the remote call returns, in one Postgres
transaction (see `crates/db` invariant: DB writes for training completion
are transactional with the billing side effect).

`LocalGpuProvider` (same activity, `APP_GPU_PROVIDER=local`) takes the same
`StartTrainingActivity` code path but calls the training core directly
in-process on the worker's own attached GPU instead of going through Modal.
Both providers call the exact same function:

```python
run_training_core(input, *, s3, s3_bucket, settings, llm_config: TenantLlmConfig)
```

defined in `apps/workers/src/activities/train_model.py`. This is the single
shared training implementation — local and remote runs execute identical
code, so there is no drift between "how training works on my laptop" and
"how training works on Modal."

## 2. Why the pure-compute boundary

A serverless GPU container is a different network than the worker's compose
network. It cannot reach `postgres:5432` or `redis:6379` inside your
docker-compose stack — those are private, internal addresses that only
exist for the worker/API/DB containers on the same Docker network. A
Modal-hosted function runs on Modal's infrastructure, reachable only via the
public internet, so it can only ever talk to services that are themselves
reachable from the public internet (S3, an LLM API endpoint).

This shaped a hard design rule: **the remote training function is pure
compute.** It receives everything it needs as a plain-data payload (dataset
S3 path, base model name, hyperparameters, a resolved `llm_config`) and
returns everything the worker needs (`adapter_path`, `adapter_size_bytes`,
`metrics`) as a plain dict. It never opens a DB connection, and it forces a
log-only metrics sink (`APP_METRICS_BACKEND=log`, set in `modal_app.py`
before settings load) so it never streams to Redis either — training
metrics land in the container's log output instead, and progress reaches
the worker only as coarse `activity.heartbeat()` calls, not live per-step
metrics. All persistence — job status transitions, the `models` row, the
billing outbox entry — is owned exclusively by `StartTrainingActivity` on
the worker side, which already has the DB pool.

Concretely:

- `run_training_core()` (`apps/workers/src/activities/train_model.py`) takes
  `s3`, `s3_bucket`, `settings`, and an already-resolved `llm_config:
  TenantLlmConfig` — no `db` argument, no `infra` container.
- `apps/workers/src/modal_runtime.py` is deliberately DB-free: `build_settings()`
  just loads `WorkerSettings` from env, `build_s3_client()` builds a boto3
  client. Nothing in that module can construct a DB connection.
- The tenant's LLM credentials are resolved from Postgres *on the worker
  side*, before the payload is built (`StartTrainingActivity.run` calls
  `get_tenant_llm_config(db=self.infra.db, ...)`), then serialized
  (`dataclasses.asdict`) into the spawn payload. The remote side reconstructs
  `TenantLlmConfig(**payload["llm_config"])`. Secrets travel once, in the
  spawn call, never via a DB connection from Modal.

This is also why `LocalGpuProvider` and `ModalGpuProvider` share
`run_training_core` instead of each having their own training logic:
whatever bug-for-bug behavior you get locally is exactly what runs in the
cloud, and any future provider (see §5) only needs to be able to reach S3
and the judge LLM — nothing else.

## 3. Why deployed-app + spawn/poll (not ephemeral `app.run()`)

Modal supports two ways to invoke a function: an **ephemeral app** (`modal run`
/ `app.run()` as a context manager, used mostly for quick scripts) and a
**deployed app** (`modal deploy`, then invoked from anywhere via
`Function.from_name(...)`).

BrainDrain uses the deployed-app pattern exclusively
(`apps/workers/modal_app.py`, deployed via `make modal-deploy`):

```python
fn = modal.Function.from_name(settings.modal_app_name, settings.modal_function_name)
fc = await fn.with_options(gpu=gpu).spawn.aio(payload)
```

The reason is job lifetime. An ephemeral app's lifecycle is tied to the
client process that started it — when `app.run()`'s context manager exits
(process restart, deploy, crash, or just the `modal run` CLI process
ending), Modal tears down any calls spawned under that ephemeral app. A
training job realistically runs for tens of minutes to hours; a worker
process can be redeployed, OOM-killed, or restarted by the orchestrator at
any point during that window, and none of that should kill an in-flight GPU
job the tenant is paying for.

A **deployed** app has no such lifecycle coupling. `modal deploy` publishes
the app once, independent of any calling process. `Function.from_name(...)`
looks up that persistent, already-running deployment and `spawn.aio(payload)`
kicks off a new call against it — a `FunctionCall` whose survival has
nothing to do with the worker process that spawned it. The worker can crash
and restart minutes later; the `FunctionCall` (identified by
`fc.object_id`) keeps running on Modal's infrastructure regardless. This is
the whole reason the reservation pattern in §4 works at all.

Polling is done non-blocking:

```python
try:
    result = await fc.get.aio(timeout=0)
    break
except TimeoutError:
    activity.heartbeat()
    await asyncio.sleep(settings.modal_poll_interval_secs)
```

`timeout=0` returns immediately (raising `TimeoutError` if the call hasn't
finished) instead of blocking the worker's event loop for the entire
training run. The loop calls `activity.heartbeat()` on every empty poll so
Temporal's activity-heartbeat timeout doesn't fire during a multi-hour job,
then sleeps `APP_MODAL_POLL_INTERVAL_SECS` before checking again.

## 4. Why the reservation pattern

Training jobs are exactly the kind of "streaming/async result" operation
this repo's Correctness-Over-Convenience rules call out: the final value
(the trained adapter) is only known after a long-running async operation,
so a durable reservation has to be written *before* the worker loses
control waiting on that operation — otherwise a crash between "spawn" and
"the DB knowing about it" causes a retry to spawn a second, duplicate GPU
job (real money, twice).

`ModalGpuProvider.run_training` therefore does, in strict order:

1. **Check for an existing reservation first.** Before spawning anything,
   it reads `training_jobs.modal_call_id` for this job. If it's already
   set, that's not a fresh job — it's an activity retry or a worker restart
   recovering a job that was already spawned. It reconnects via
   `modal.FunctionCall.from_id(existing)` and skips straight to polling. No
   new GPU is provisioned, no money is spent twice.
2. **Spawn only if there's no existing reservation.**
   `fc = await fn.with_options(gpu=gpu).spawn.aio(payload)`.
3. **Persist `fc.object_id` to `training_jobs.modal_call_id` immediately,
   before entering the poll loop.** This is the reservation write. Once it
   commits, any future retry or restart will take the "recover" branch in
   step 1 instead of respawning.
4. Only then does it start polling.

For the recovery branch (step 1) to ever actually fire, the calling
workflow's `start_training` activity must be allowed to retry at least once.
`train.py`, `train_aligned.py`, and `train_reasoning.py` set
`retry_policy=workflow.RetryPolicy(maximum_attempts=2)` on the
`start_training` activity call for exactly this reason — with the previous
`maximum_attempts=1`, Temporal never retried the activity at all, so a
crashed or timed-out worker orphaned the in-flight Modal `FunctionCall`
instead of reconnecting to it. `maximum_attempts=2` gives one retry, which
is enough to hit the `existing` branch above and resume polling rather than
respawning.

The column is `crates/db/src/migrations/015_training_jobs_modal_call_id.sql`
— a single nullable `TEXT` column, `NULL` for jobs run on `LocalGpuProvider`
where there's no remote call to reconnect to. The full-evaluation path uses a
separate `evaluations.modal_call_id`
(`016_evaluations_modal_call_id.sql`), since evaluations key on
`evaluations.id`, not `training_jobs.id`.

**Shared column across activity types → tagged reservations.** The iterative
workflow reuses the single `training_jobs.modal_call_id` for *both*
`train_sft_round` and `evaluate_holdout`, across every round (they run
sequentially, so at most one Modal call is ever in flight per job). To make
that safe, the stored value is tagged `"<function_name>:<call_id>"` and
`ModalGpuProvider._recoverable_call_id` recovers a reservation **only when the
tag matches the function about to run**. This closes a real corruption bug:
`TrainIterativeWorkflow` deliberately *tolerates* a failed `evaluate_holdout`
(e.g. no `_val.jsonl` split) and continues; without tagging, the stale
holdout `modal_call_id` would be "recovered" by the next round's
`train_sft_round`, which would return the holdout result (or its error)
instead of training — silently skipping a round or killing the job. Iterative
methods also `SET modal_call_id = NULL` after a *successful* call so a later
same-function round doesn't recover the finished one. Bare (untagged) values
written by the pre-tagging release are treated as legacy single-shot-training
reservations, so training jobs in flight across the upgrade deploy still
recover correctly.

**Honest caveat:** there is still a narrow crash window between step 2
(spawn succeeds, Modal is now running the job and billing for it) and step 3
(the `UPDATE` commits). If the worker process dies in exactly that window,
the reservation write never happens, and a subsequent retry will not find an
`modal_call_id` to recover — it will spawn a second job. This window is a
single unconditional `UPDATE` right after an `await`, so it's about as small
as it can be made without a two-phase commit against Modal itself, but it is
not zero. This is a known, accepted gap, not a solved one — call it out
explicitly rather than claiming full exactly-once semantics.

Modal retains a spawned call's result for approximately **7 days** after
completion. That means recovery via `FunctionCall.from_id(...).get.aio()`
works for any retry within that window, even long after the original worker
process is gone; beyond 7 days an old `modal_call_id` may no longer resolve
to a fetchable result.

## 5. How to add a new GPU provider (e.g. RunPod)

The provider contract is a `Protocol`, not a base class, and it's
deliberately narrow (`apps/workers/src/gpu_provider.py`):

```python
class GpuProvider(Protocol):
    async def run_training(
        self, *, tenant_id, training_job_id, dataset_path, base_model,
        method, mode, hyperparams, gpu_class, llm_config: dict,
    ) -> dict:
        """Returns {adapter_path, adapter_size_bytes, metrics}."""
```

To add a provider:

1. Write a class implementing `run_training(**data, llm_config) -> dict`.
   It should call (or remotely invoke something that calls)
   `run_training_core(...)` from `src.activities.train_model` — do not
   reimplement the training logic. The only hard requirement on the remote
   side is that it can reach S3 (to pull the dataset and push the adapter)
   and, for `aligned`/`reasoning` modes, the judge LLM endpoint. It needs no
   database and no Redis connection.
2. If the new provider is also request/poll based (most serverless GPU
   platforms are), follow the reservation pattern from §4: persist whatever
   the remote job's tracking id is to a durable column before polling, and
   check for an existing one before spawning. (`modal_call_id` is Modal-
   specific; a RunPod provider would want its own column, or a
   provider-prefixed value in a more generic column if you want to share
   one.)
3. Add one branch to `create_gpu_provider()`:

   ```python
   elif provider_name == "runpod":
       return RunpodGpuProvider(infra)
   ```
4. Add whatever config the provider needs to `WorkerSettings`
   (`apps/workers/src/config.py`), following the `modal_*` fields as a
   template, and set `APP_GPU_PROVIDER=runpod`.

No workflow, activity, or DB migration changes are needed beyond the
provider's own reservation column — `StartTrainingActivity` only ever calls
`self.gpu_provider.run_training(...)` through the `GpuProvider` protocol.

## 6. Configuration

All cloud-GPU config lives in `apps/workers/src/config.py`
(`WorkerSettings`, prefix `APP_`) and `apps/workers/src/constants.py`.

| Variable | Default | Purpose |
|---|---|---|
| `APP_GPU_PROVIDER` | `local` | `local` runs training in-process on the worker's own GPU. `modal` dispatches to the deployed Modal function. |
| `APP_MODAL_APP_NAME` | `platform-training` | Name of the deployed Modal app (`modal.App("platform-training")` in `modal_app.py`). Must match what `make modal-deploy` published. |
| `APP_MODAL_FUNCTION_NAME` | `train` | Name of the function within that app to invoke (`@app.function` decorated `train`). |
| `APP_MODAL_SECRET_NAME` | `platform-training-secrets` | Name of the Modal secret the deployed function reads env vars from. Informational on the worker side (the secret is bound in `modal_app.py`, not passed at call time) — keep it in sync if you rename the secret. |
| `APP_MODAL_POLL_INTERVAL_SECS` | `15` | How often `ModalGpuProvider` polls `FunctionCall.get(timeout=0)` while waiting for the remote job. Also the interval between `activity.heartbeat()` calls. |

GPU class mapping (`apps/workers/src/constants.py`, `MODAL_GPU_MAP`) translates
the platform's `gpu_class` values (also used for cost estimation via
`GPU_HOURLY_RATES`) into the GPU type string Modal expects in
`.with_options(gpu=...)`:

| `gpu_class` (canonical, lowercase) | Modal GPU string |
|---|---|
| `t4` | `T4` |
| `a10g` | `A10G` |
| `l40s` | `L40S` |
| `a10040gb` | `A100` |
| `a10080gb` | `A100-80GB` |
| `h100` | `H100` |

Keys are the canonical lowercase `gpu_class` values (matched
case-insensitively). Anything not in the map falls back to
`MODAL_DEFAULT_GPU` (`T4`). A single deployed function (`train`) serves every
GPU class — the GPU type is chosen per-call via `.with_options(gpu=...)`, not
baked into the deployed app.

### 6.1 Live training metrics on the cloud path

By default the Modal path forces `APP_METRICS_BACKEND=log` (set in
`modal_app.py` before settings load), so per-step metrics go to the container
log, not Redis. The dashboard's training stream reads
`training:metrics:<job_id>` from Redis, so on a default cloud run the SSE view
sits on **"waiting for metrics…"** for the whole job — status transitions
still land (they go through Postgres), but the live loss chart never fills.

To get live per-step metrics on the cloud path, point the remote at a
Redis the Modal network can reach (e.g. Upstash) — never a compose-internal
host:

- `APP_METRICS_BACKEND=redis`
- `APP_REDIS_URL=rediss://…` (a public `rediss://` URL, in the Modal secret)

This is verified end-to-end (§10). The local path (`APP_GPU_PROVIDER=local`)
already streams live metrics with no extra setup.

## 7. Modal secret setup

The deployed function binds one Modal secret,
`modal.Secret.from_name("platform-training-secrets")`
(`apps/workers/modal_app.py`). Since the remote container calls
`WorkerSettings()` (via `build_settings()` in `modal_runtime.py`) to load its
config, that secret must supply every environment variable
`WorkerSettings` requires to construct successfully — including ones the
remote code never actually uses.

Required keys:

- `APP_S3_ENDPOINT`, `APP_S3_ACCESS_KEY`, `APP_S3_SECRET_KEY`, `APP_S3_BUCKET`,
  `APP_S3_REGION` — must point at cloud-reachable object storage (see §9;
  `http://minio:9000` will not resolve from Modal's network).
- `APP_LLM_API_BASE_URL`, `APP_LLM_API_KEY`, `APP_LLM_MODEL` — used as the
  fallback judge LLM config for DPO/GRPO reward scoring. (A tenant-specific
  key travels in the spawn payload instead — see §10.)
- `APP_HF_TOKEN` — HuggingFace token for gated/private base models.
- `APP_DATABASE_URL` — **a syntactically valid placeholder that is never
  connected to.** `WorkerSettings.database_url` is a required field with a
  validator (`database_url_must_be_postgresql`) that only checks the string
  starts with `postgresql://` — it never opens a connection. The remote
  container has no DB access and none of `run_training_core`'s code path
  touches `settings.database_url`. Without *some* value here,
  `WorkerSettings()` simply fails to construct in the container. A
  placeholder like `postgresql://unused:unused@unused:5432/unused` is
  sufficient and intentional — do not point this at a real database.
- `APP_METRICS_BACKEND` — optional. `modal_app.py`'s `train()` does
  `os.environ.setdefault("APP_METRICS_BACKEND", "log")` before
  `build_settings()`, so the remote container defaults to the log-only sink.
  To stream live per-step metrics instead, set `APP_METRICS_BACKEND=redis`
  **and** a publicly reachable `APP_REDIS_URL` (an Upstash `rediss://` URL, or
  any Redis Modal's network can reach — never a compose-internal
  `redis://localhost:6379`) in the secret. Leaving it unset keeps the safe log
  default.

Create the secret with the Modal CLI (values below are placeholders — do not
paste real credentials into shell history or CI logs):

```bash
modal secret create platform-training-secrets \
  APP_S3_ENDPOINT=https://s3.amazonaws.com \
  APP_S3_ACCESS_KEY=<your-s3-access-key> \
  APP_S3_SECRET_KEY=<your-s3-secret-key> \
  APP_S3_BUCKET=<your-bucket> \
  APP_S3_REGION=us-east-1 \
  APP_LLM_API_BASE_URL=https://api.openai.com/v1 \
  APP_LLM_API_KEY=<your-llm-api-key> \
  APP_LLM_MODEL=gpt-4o-mini \
  APP_HF_TOKEN=<your-hf-token> \
  APP_DATABASE_URL=postgresql://unused:unused@unused:5432/unused
```

Re-running `modal secret create` with the same name and `--force` updates an
existing secret's values (see `modal secret create --help`); the deployed
function picks up new secret values on its next cold start.

### 7.1 Object storage backends (AWS S3 / MinIO / Cloudflare R2 / other)

The platform never hard-codes AWS. Both the worker (`src/infra.py`) and the
remote Modal container (`src/modal_runtime.py`) build their S3 client through
one shared factory, `src/s3_client.create_s3_client()`, so **any
S3-compatible store works by config alone** — set `APP_S3_ENDPOINT`,
`APP_S3_ACCESS_KEY`, `APP_S3_SECRET_KEY`, `APP_S3_BUCKET`, `APP_S3_REGION` and
nothing else changes. The factory's `S3_COMPAT_CONFIG` (SigV4, path-style
addressing, and `request/response_checksum_calculation="when_required"`) is a
single setting that is safe for AWS, MinIO, and R2 alike — it disables the
botocore ≥1.36 default CRC32 upload checksum that otherwise makes plain
`PutObject` fail against non-AWS stores.

| Backend | `APP_S3_ENDPOINT` | `APP_S3_REGION` |
|---|---|---|
| AWS S3 | `https://s3.<region>.amazonaws.com` (or omit for default) | real region, e.g. `us-east-1` |
| MinIO (local/self-host) | `http://minio:9000` | `us-east-1` |
| Cloudflare R2 | `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` | `auto` |

**Cloudflare R2 is the recommended free option** for the cloud-GPU E2E if you
don't run AWS: its free tier is permanent (10 GB storage, 1M Class-A + 10M
Class-B ops/month) and — most relevant here — **egress is always free**, so
Modal pulling the dataset and pushing the LoRA adapter costs nothing in
bandwidth. It satisfies the "cloud-reachable S3" prerequisite in §9 (unlike a
localhost MinIO, which Modal's network can't reach).

R2 credential setup: Cloudflare dashboard → **R2** → **Manage R2 API Tokens**
→ create a token (Object Read & Write is enough) → copy the **Access Key ID**
and **Secret Access Key** (shown once) and use them as `APP_S3_ACCESS_KEY` /
`APP_S3_SECRET_KEY` against the `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`
endpoint with `APP_S3_REGION=auto`. Buckets created with an `eu`/`fedramp`
jurisdiction use the matching `https://<ACCOUNT_ID>.<jurisdiction>.r2.cloudflarestorage.com`
endpoint.

> **Note:** the Rust control plane's object-storage client
> (`crates/storage/src/s3.rs`, used for user document uploads) gets the
> equivalent treatment for the same reason — it already forces path-style for
> custom endpoints and now sets `request_checksum_calculation` /
> `response_checksum_validation` to `WhenRequired` on the `aws_sdk_s3::Config`
> builder (aws-sdk-s3 has the same default-checksum behavior as botocore
> ≥1.36). So both planes — worker training I/O and control-plane uploads —
> work against AWS S3, MinIO, and R2 by config alone.

## 8. Deploy

```bash
make modal-deploy
```

which runs:

```bash
cd apps/workers && uv run modal deploy modal_app.py
```

This publishes (or republishes) the `platform-training` app and its `train`
function to Modal. Deploying does **not** require Modal-specific pip extras
on the machine you deploy *from* other than the `modal` package itself
(`uv sync --extra gpu-cloud` in `apps/workers/pyproject.toml`) — the image
your training code actually runs in is built entirely from the
`modal.Image` spec in `modal_app.py` (Modal builds and caches this image on
its own infrastructure, not locally).

Note `modal_app.py`'s image ends with `.add_local_python_source("src")` —
this must be the last layer in the image chain, since Modal's local-source
mount is layered on top of (not baked into) the built image; adding pip
layers after it would not see the mounted source. If you add new imports
under `apps/workers/src/` that the remote `train` function needs, no image
change is required — the whole `src` package is mounted, but check whether
the new import pulls in a package not already in the image (see below).

The image's `pip_install(...)` list is **not** a literal copy of any single
`pyproject.toml` dependency group. It is: the pyproject `[ml]` extra
(unsloth, transformers, datasets, trl, peft, accelerate, bitsandbytes,
pynvml) **minus** `distilabel` (that's data-generation tooling, never
imported by the training path), **plus** `temporalio`, `asyncpg`, `redis`,
`boto3`, `httpx`, `pydantic`, `pydantic-settings` from the base
`dependencies` list — these are pulled in transitively at import time by
`src.activities.stubs` / `src.activities.train_model` (via `src.infra`,
which does `import asyncpg` and `import redis.asyncio`) and `from
temporalio import activity`, even though the remote code path never
actually calls Postgres or Redis. Without them the remote module import
fails with `ModuleNotFoundError` before training starts. If you add a new
*pip dependency* anywhere in the modules the remote `train` function
imports (directly or transitively), add it to the `image =
modal.Image...pip_install(...)` list in `modal_app.py` and redeploy — do
not assume the `[ml]` extra alone covers it.

Redeploy any time `modal_app.py`, `modal_runtime.py`, or anything under
`apps/workers/src/` that the remote path imports changes. The deployed
function is versioned by Modal on each `modal deploy`; in-flight jobs
started under a previous deployment continue running under whichever image
version they were spawned with.

## 9. Cheap end-to-end runbook (~$30 budget)

This validates the full remote path — spawn, poll, reservation, adapter
landing in S3, `training_jobs` reaching `completed` — for a few dollars.

**Prerequisite you must satisfy first:** the remote GPU container only has
network access to the public internet, not to your local docker-compose
network. **A full train → adapter → S3 run requires cloud-reachable object
storage** — real AWS S3 (or another provider's S3-compatible endpoint that
is itself reachable from the public internet, e.g. Cloudflare R2, Backblaze
B2). A local/compose MinIO bound to `localhost:9000` is **not** reachable
from Modal; if `APP_S3_ENDPOINT` in the secret points at a local address,
the remote job will fail at the dataset-download step. Stand up (or reuse)
a real cloud bucket for this runbook, and populate the training dataset
into it beforehand (or via the normal upload → parse → refine pipeline
pointed at that bucket).

Steps:

1. Set `APP_GPU_PROVIDER=modal` in the worker's environment (`.env` or your
   deployment config) for the worker process(es) handling the
   `ml-pipeline-gpu` queue (or `ml-pipeline` in single-queue dev mode).
2. Create the Modal secret as in §7, pointing `APP_S3_*` at the
   cloud-reachable bucket from the prerequisite above.
3. `make modal-deploy`.
4. Kick off one training job through the normal pipeline (dashboard or API)
   with:
   - A tiny base model (~0.5B–1B parameters — e.g. a small Qwen2.5 or SmolLM2
     checkpoint) to keep both compute time and cost minimal.
   - `mode = "quick"` (SFT-only — fastest, avoids the DPO/GRPO judge-LLM
     passes for this smoke test).
   - A handful of dataset rows (10-50 examples is enough to exercise the
     full path without a long training run).
   - `gpu_class` set to `T4` or `A10G`/`A10` (cheapest classes in
     `MODAL_GPU_MAP` / `GPU_HOURLY_RATES`).
5. Watch the worker logs: `"Spawning Modal training (job=..., gpu=..., model=...)"`
   followed by the reservation UPDATE, then periodic poll/heartbeat activity
   until `"Modal training complete (job=...)"`.
6. Verify:
   - The adapter directory exists under the expected S3 prefix
     (`s3_paths.adapter_training_prefix(tenant_id, job_id)`).
   - `training_jobs.status = 'completed'`, `training_jobs.modal_call_id` is
     set, `actual_cost` is populated.
   - A row was inserted into `models` for this job.
7. Expected cost: a tiny model, a handful of examples, and a single SFT pass
   on a T4/A10 typically completes in well under 10 minutes of GPU time —
   at $0.80-$1.20/hr (see `GPU_HOURLY_RATES`), that's a few dollars at most,
   comfortably inside a ~$30 budget even accounting for a couple of retries
   or a slightly larger smoke-test model.

## 10. Known limitations / deferred work

- **Iterative training rounds and full evaluation now run on Modal too.**
  `TrainSftRoundActivity`, `EvaluateHoldoutActivity` (used by
  `TrainIterativeWorkflow`), and `RunEvaluationActivity` all go through
  `GpuProvider` — each has a pure-compute core (`run_sft_round_core`,
  `run_evaluate_holdout_core`, `run_evaluation_core`) shared by the local
  and remote paths, a dedicated deployed Modal function (`train_sft_round`,
  `evaluate_holdout`, `run_evaluation`), and a `ModalGpuProvider` method with
  the same spawn/poll/reservation flow as `run_training`. With
  `APP_GPU_PROVIDER=modal`, the single-shot **and** iterative training and
  evaluation workflows are all cloud-GPU-capable. `ExportGgufActivity`
  remains local-only (it packages an already-trained adapter; it is not a
  training/eval GPU workload).
- **A slim (`--extra gpu-cloud`, no CUDA) worker can serve the whole
  training + evaluation pipeline against Modal** — `StartTrainingActivity`,
  `TrainSftRoundActivity`, `EvaluateHoldoutActivity`, and
  `RunEvaluationActivity` all offload. Only `ExportGgufActivity` still needs
  local compute, so keep a `Dockerfile.gpu`-based worker on `ml-pipeline-gpu`
  if your pipeline exports GGUF. Standalone evaluation started from the Rust
  control plane (`EvaluateWorkflow` via `evaluation_service.rs`) does not yet
  forward a `gpu_class`, so remote eval on that path uses `MODAL_DEFAULT_GPU`
  (A10) — fine for the two 4-bit models it loads; the automatic
  post-training eval in `FullPipelineWorkflow` forwards the training
  `gpu_class`.
- **Holdout eval correctness (fixed while offloading).** Two pre-existing
  bugs in `_evaluate_on_holdout` / the holdout adapter load surfaced only when
  the path was actually exercised on Modal: (1) the iteration's saved adapter
  is now loaded with `PeftModel.from_pretrained` (the old
  `attach_adapter()` + `load_adapter("default")` created a fresh random
  adapter and then failed to load real weights into the same name — a
  state_dict mismatch); (2) Unsloth's trainer init requires a `train_dataset`
  even for an eval-only run (`fix_zero_training_loss` calls `len()` on it), so
  the eval-only `SFTTrainer` is given the val set as its (never-trained)
  `train_dataset`. Before these fixes the holdout eval always raised and
  `TrainIterativeWorkflow`'s tolerate-and-proxy fallback silently compared
  *train* loss for early stopping; it now produces a real held-out
  `eval_loss`.
- **Only S3 (and the judge LLM endpoint) must be cloud-reachable.** No other
  service needs to be exposed to the public internet for the cloud-GPU path
  to work. Live per-step metrics streaming to Upstash (`APP_METRICS_BACKEND=redis`
  + a `rediss://` `APP_REDIS_URL` in the Modal secret) is verified end-to-end:
  a remote `evaluate_holdout` on Modal streamed `eval_begin`/`eval_end` events
  to an Upstash stream (`training:metrics:<job_id>`) read back over the public
  endpoint.
- **A tenant-supplied custom LLM key is not read from the Modal secret.**
  `StartTrainingActivity` resolves `TenantLlmConfig` from Postgres (per
  tenant) *before* building the spawn payload, then serializes it
  (`dataclasses.asdict(llm_config)`) into `payload["llm_config"]`. This means
  a tenant's own LLM API key, if configured, travels as part of the Modal
  function call's input (stored by Modal as the `FunctionCall`'s input
  data) rather than living in the static `platform-training-secrets` secret.
  The static secret's `APP_LLM_*` values are only the worker-level fallback
  used when a tenant has no custom config. This is a reasonable per-call
  boundary (keys are tenant-scoped, not baked into the shared deployed
  function) but be aware that tenant LLM credentials pass through Modal's
  input-storage layer, not just Modal's declared-secret mechanism, when
  reasoning about where that data flows.
- **The crash window described in §4** (spawn succeeds, then the process
  dies before the reservation `UPDATE` commits) is real, not eliminated,
  and would cause exactly one duplicate spawn on the next retry.
