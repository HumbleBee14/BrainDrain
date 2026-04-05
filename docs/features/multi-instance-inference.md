# Multi-Instance Inference Control Plane

**PR:** #26  
**Problem:** The platform originally assumed one global inference server URL.
That was acceptable for a first deployment path, but not for a serious
production control plane. It meant:
- deployments were effectively tied to one serving node
- inference routing depended on process-global backend config
- adapter capacity was enforced per process, not per serving fleet
- draining, retirement, and health-based routing were not first-class

## What changed

BrainDrain now has an explicit **inference control plane**.

The API tracks inference servers in a dedicated registry, binds deployed
models to specific instances, and routes deploy / infer / undeploy through the
assigned instance instead of assuming one global server.

At a high level:

```
Admin registers inference instance
  -> API stores backend type, base model, URL, health, lifecycle, capacity
  -> deploy claims a compatible healthy instance with free adapter slots
  -> model stores inference_instance_id
  -> inference resolves the model's assigned instance
  -> undeploy unloads from that exact instance and releases capacity
```

## Why this matters

- **Scales past one GPU node** — you can add more serving instances without
  rewriting deployment and inference logic
- **Avoids hard coupling** — the control plane is no longer structurally tied
  to one backend URL or one engine implementation
- **Improves correctness** — deploy, inference, and undeploy all agree on the
  authoritative instance for a model
- **Makes operations safer** — instances can be marked `draining` or
  `retired`, and unhealthy nodes are excluded from placement

## Core ideas

### 1. Instance registry

Inference servers are stored in `inference_instances` with:
- `backend_type`
- `base_url`
- `base_model`
- `max_adapters`
- `active_adapter_count`
- `health_status`
- `lifecycle_state`

This turns serving capacity into explicit control-plane state instead of
hidden process configuration.

### 2. Authoritative model binding

Each deployed model now stores a nullable `inference_instance_id`.

That means the platform has one relational source of truth for:
- where a model is deployed
- which backend instance should serve inference
- which instance should receive undeploy requests

This is better than relying on incidental JSON config alone.

### 3. Placement with capacity accounting

When multi-instance mode is enabled, deployment:
- finds a compatible instance by backend type and base model
- claims adapter capacity
- marks the model `deploying` atomically
- loads the adapter on the chosen instance
- finalizes the binding

If the claim loses a race, the slot is released cleanly.

### 4. Instance-aware inference routing

Inference no longer assumes a single global backend when a model is bound to
an instance. The API:
- looks up the model
- resolves its assigned instance
- builds or reuses the matching backend client
- sends the request to that instance

Single-instance mode still works when the feature flag is off.

### 5. Health and lifecycle management

The control plane now includes:
- periodic health probes
- reconciliation of cached adapter counts against model state
- lifecycle states such as `ready`, `draining`, and `retired`

This gives the platform safe operational controls for rolling maintenance and
recovering from stale state.

## Operational model

### Feature flag

The new routing path is gated by:

```text
deployments.multi_instance.enabled
```

Behavior:
- flag off -> legacy single-instance path via `INFERENCE_SERVER_URL`
- flag on -> deployments must use registered inference instances

### Admin API

Owners can manage inference instances via:
- `GET /api/v1/admin/inference-instances`
- `POST /api/v1/admin/inference-instances`
- `PATCH /api/v1/admin/inference-instances/{id}/lifecycle`
- `DELETE /api/v1/admin/inference-instances/{id}`

### Lifecycle states

- `ready` — accepts new placements
- `draining` — serves existing deployments but accepts no new ones
- `retired` — removed from scheduling; should be empty before deletion

## Benefits to future evolution

This PR is intentionally scoped to the control plane, not full autoscaling.
That is a strength, not a limitation.

Because the code now has:
- backend abstraction
- instance registry
- explicit model binding
- health / lifecycle states

future additions become much easier:
- auto-registration
- autoscaling / fleet managers
- Kubernetes operators
- cross-region placement
- richer scheduling policies

Those can be layered on top without rewriting deploy and inference flows.

## Key fixes included during review

The final version also tightened several correctness edges:
- prevented double-deploy races
- moved undeploy adapter unload after DB commit
- preserved shared circuit breaker behavior for single-instance fallback
- removed TOCTOU in instance lifecycle/delete transitions
- returned `201 Created` on instance registration
- defaulted instance metadata to `{}` instead of `null`

## Files

- `crates/db/src/migrations/013_inference_instances.sql` — instance registry schema
- `crates/api/src/repositories/inference_instance_repo.rs` — repository and scheduling operations
- `crates/api/src/services/inference_instance_service.rs` — instance management and health checks
- `crates/api/src/services/deployment_service.rs` — multi-instance deploy / undeploy logic
- `crates/api/src/routes/inference.rs` — instance-aware inference routing
- `crates/api/src/routes/admin_instances.rs` — admin management routes
- `crates/api/src/app_state.rs` — backend resolution and shared backend access
- `crates/shared/src/enums.rs` — health and lifecycle enums
