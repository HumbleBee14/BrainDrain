# System Architecture

> Current production architecture of BrainDrain after the full production-excellence hardening pass.

## 1. Runtime Topology

```text
                         +----------------------+
                         |      Frontend        |
                         |  Next.js / React     |
                         +----------+-----------+
                                    |
                                    v
                         +----------------------+
                         |   Rust API / Axum    |
                         | Control plane        |
                         +----+-----------+-----+
                              |           |
             +----------------+           +----------------+
             |                                             |
             v                                             v
   +----------------------+                     +----------------------+
   | PostgreSQL + RLS     |                     | Redis               |
   | billing outbox       |                     | rate limiting       |
   | idempotency          |                     | delivery / caching  |
   | inference instances  |                     | pub-sub / streams   |
   +----------+-----------+                     +----------------------+
              |
              v
   +----------------------+
   | PgBouncer (prod)     |
   +----------------------+

   +----------------------+                     +----------------------+
   | Temporal             |<------------------->| Python workers       |
   | durable workflows    |                     | parse / refine /     |
   | retries / heartbeats |                     | train / eval / export|
   +----------------------+                     +----------+-----------+
                                                             |
                                                             v
                                                    +------------------+
                                                    | S3 / MinIO       |
                                                    | documents,       |
                                                    | datasets,        |
                                                    | adapters, exports|
                                                    +------------------+
```

## 2. Core Service Boundaries

### Rust API (`crates/api`)

Responsible for:
- authentication and team membership resolution
- RBAC and API key auth
- project, dataset, model, billing, and deployment APIs
- idempotency enforcement for mutating endpoints
- durable billing outbox writes and relay lifecycle
- inference request routing
- inference instance registry and placement control plane

Key patterns:
- `Route -> Service -> Repository`
- request identity resolved once in auth middleware and stored in request extensions
- explicit SQL via `sqlx`
- environment-driven configuration
- feature flags behind a provider abstraction

### Python Workers (`apps/workers`)

Responsible for:
- document parsing
- chunking and synthetic dataset generation
- model training
- evaluation
- export workflows

Key patterns:
- Temporal activities for long-running or retryable work
- pluggable backend registries for parser, chunker, LLM provider, judge, metrics, and training engine
- direct shared-database writes only where transactional coupling is required, such as training billing outbox

### Frontend (`apps/web`)

Responsible for:
- dashboard and project UX
- training/deployment workflows
- settings and billing views
- inference playground

## 3. Data and Control Flows

### Product Pipeline

```text
Upload docs
  -> parse
  -> refine into training pairs / dataset
  -> train
  -> evaluate
  -> deploy
  -> infer
```

### Billing Flow

```text
authoritative event occurs
  -> write billing_outbox row in same DB transaction when required
  -> relay claims rows with advisory lock
  -> persist into billing_events ledger
  -> Stripe / reporting consume the ledger
```

### Auth and Idempotency Flow

```text
request
  -> auth middleware verifies identity and role
  -> AuthOutcome stored in extensions
  -> idempotency middleware reads verified identity
  -> handler reads AuthenticatedUser from extensions
```

## 4. Multi-Tenancy

Tenant isolation is enforced at multiple layers:
- repository queries scoped by `tenant_id`
- PostgreSQL RLS on tenant-scoped tables
- connection checkout resets tenant context
- API keys scoped to tenant and model
- S3 paths namespaced by tenant/project/model
- Temporal workflow inputs carry tenant identity

Global infrastructure tables such as `inference_instances` are intentionally not tenant-scoped.

## 5. Serving Architecture

Inference is now both pluggable and instance-aware.

### Backend Abstraction

Supported control-plane backend types:
- `vllm`
- `tgi`
- `sglang`

The API does not hard-code one engine. It builds the correct backend implementation from:
- global single-instance config
- or the assigned `inference_instance`

### Multi-Instance Control Plane

The platform tracks inference servers in `inference_instances` and binds deployed models to a nullable `models.inference_instance_id`.

This enables:
- compatible-instance placement by backend type and base model
- slot reservation using DB-backed capacity accounting
- health-based scheduling
- draining and retirement of instances
- per-instance routing for deploy, inference, and undeploy
- reconciliation of cached adapter counts against model state

Single-instance mode still works when the feature flag is off.

**Verified state:** the backend abstraction, multi-instance placement, and
health reconciliation are implemented with unit tests. vLLM (with the S3
LoRA resolver, see `infra/serving/README.md`) is the primary exercised
backend; TGI and SGLang implement the same trait but are less exercised.
Neither the serving path nor the CI/CD pipeline (build-and-validate only,
`push: false` by design — see `docs/DEPLOYMENT.md`) has been proven against
sustained production traffic yet.

### Serving Request Path

```text
API key request
  -> resolve deployed model
  -> inject the model's trained system prompt as default if the caller sent none
  -> resolve assigned inference instance if present
  -> build or reuse backend for that instance
  -> call backend OpenAI-compatible API
  -> meter usage via durable billing path
```

The default-system-prompt step reuses the optional per-guide `system_prompt`
the model was trained under (stored in `deployment_config` at deploy time), so
training and serving stay on the same chat template. A caller-supplied `system`
message always takes precedence. See `docs/DATA_PIPELINE.md` for the full path.

## 6. Production Hardening That Now Exists

### Reliability
- auth middleware with preserved error semantics
- API idempotency for mutating routes
- durable billing outbox
- training billing on the same outbox model
- PITR scripts and restore path
- PgBouncer for production connection topology
- release pre-check scripts
- inference instance health probes and reconciliation

### Operational Controls
- feature flags with static and guarded Unleash providers
- idempotency cleanup
- outbox relay drain on shutdown
- deploy stale-state cleanup
- instance lifecycle states: `ready`, `draining`, `retired`

### Developer Safety
- Rust/TypeScript binding generation
- Rust/Python shared constant sync
- strict clippy / test gates

## 7. What Is Intentionally Not In Scope Yet

These are future scale extensions, not missing core architecture:
- inference node auto-provisioning
- Kubernetes operator
- cross-region placement
- autoscaling GPU fleet management
- full Unleash strategy evaluation

The current code is designed so those can be added without rewriting the existing control plane.

## 8. Source References

Primary implementation references:
- `crates/api/src/auth.rs`
- `crates/api/src/services/idempotency.rs`
- `crates/api/src/services/billing_outbox.rs`
- `crates/api/src/services/deployment_service.rs`
- `crates/api/src/services/feature_flags.rs`
- `crates/api/src/services/inference_instance_service.rs`
- `crates/api/src/routes/inference.rs`
- `crates/api/src/repositories/inference_instance_repo.rs`
- `apps/workers/src/activities/train_model.py`
