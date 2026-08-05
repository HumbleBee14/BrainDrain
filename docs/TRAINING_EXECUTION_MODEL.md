# Training execution model — the in-process problem and the proposed fix

> **Status:** problem confirmed by reading the code, fix proposed, not implemented. Written 2026-08-04 while responding to the Stage 3 review. The immediate trigger was the on-policy GPU split, but that turned out to be the third symptom of one root cause, and the other two are worse.

## 1. Summary

`LocalGpuProvider` runs training **in the Temporal worker's own process and on its event loop**. `ModalGpuProvider` runs the same training core in a fresh remote container per job. Those are two different execution models sharing one function, and every property the Modal path gets structurally — process isolation, cancellability, a clean allocator, a per-job device set — the local path has to emulate, and mostly cannot.

The proposal is to make the local path launch the trainer as a **child process**, so both providers implement the same contract: hand a payload to a fresh process, poll it, collect a result, cancel it by killing it.

## 2. The immediate symptom: on-policy cannot confine the student

On-policy distillation puts a served teacher and a training student in one container, on separate cards. The teacher is a `trl vllm-serve` **subprocess**, so it gets its own environment and its own `CUDA_VISIBLE_DEVICES` — that half is easy. The student trains *in the calling process*, and confining it requires setting `CUDA_VISIBLE_DEVICES` before that process has ever touched CUDA, because the CUDA runtime reads the variable once, at initialisation, and ignores it forever after.

On Modal that is free: fresh container, fresh process, one job, nothing has touched CUDA. On the local path it is not, because `LocalGpuProvider.run_training` in [gpu_provider.py](../apps/workers/src/gpu_provider.py) calls `run_training_core` directly in the long-lived worker. A worker that has already trained anything has a frozen device list, so:

- `_reserve_student_devices` sets the variable and it does nothing
- `torch.cuda.device_count()` still reports every card
- the run is **refused non-retryably** rather than placing a 32B teacher and a student's optimizer state on one card

Refusing is the correct behaviour given the constraint — the alternative is an out-of-memory kill on the first long sequence. It is still a narrow path: on-policy succeeds locally only on a worker that has not yet done GPU work.

## 3. The root cause, and the two consequences that are worse

### 3.1 Training blocks the worker's event loop

`run_training_core` is `async def`, but `trainer.train()` is a synchronous multi-hour call made directly inside it. For the duration, nothing else in that worker process makes progress — no other activity, no coroutine, no timer.

There is a per-step `safe_heartbeat` (`safe_heartbeat(f"step={current_step}/{max_steps}")` in [train_model.py](../apps/workers/src/activities/train_model.py)) against a **five-minute** `heartbeat_timeout` (`train_heartbeat()` in [timeouts.py](../apps/workers/src/timeouts.py), applied to the training activity in [workflows/train.py](../apps/workers/src/workflows/train.py)). Whether that heartbeat still reaches the server from a blocked event loop depends on Temporal SDK internals that have **not been verified** — treat it as suspect, not as a confirmed failure. It should be measured before the fix is designed around it, because if heartbeats do not flush, long local training already fails at five minutes regardless of everything else here.

### 3.2 A native crash takes down the whole worker

A CUDA out-of-memory abort, or a segfault in a fused kernel, is not a Python exception. It kills the process — and with it every other activity that worker was hosting, not just the offending job. Job isolation on the local path is therefore zero: one tenant's oversized run can end another tenant's parse.

### 3.3 Cancellation cannot reach the trainer

Temporal delivers cancellation to a coroutine. There is no coroutine to deliver it to inside `trainer.train()`. The Modal path handles this properly — `ModalGpuProvider._run_remote` catches `CancelledError` and cancels the in-flight call so it stops billing immediately. The local path has no equivalent: the GPU runs to the end of the training loop no matter what the user clicked.

### 3.4 Two smaller ones, for completeness

- **The allocator never fully resets between jobs.** PyTorch's caching allocator and fragmentation persist for the life of the process, so a heavy job degrades every job that follows it on the same worker.
- **Library conflicts have to be worked around in-process.** Unsloth cannot coexist with the vLLM the on-policy teacher needs, which is why the strategy carries a `required_engine` attribute consulted before the engine is resolved. With a child process this is a choice of entrypoint, not a constraint the code has to route around.

## 4. Proposed fix: run the trainer in a child process

Launch the trainer as `python -m <training entrypoint> <payload path>`:

| Concern | Mechanism |
| --- | --- |
| Device set | `CUDA_VISIBLE_DEVICES` in the child's environment — the parent never touches CUDA, so the variable always means something |
| Payload | A JSON file in a temp dir; the child reads it and needs no other arguments |
| Result | Written by the child to a result file; the parent reads it on exit code 0 |
| Failure | Non-zero exit code plus captured stderr tail becomes the activity's error, mapped the way remote failures already are |
| Progress | The Redis progress stream the trainer already writes to — unchanged |
| Heartbeats | The parent heartbeats while waiting, on an unblocked event loop |
| Cancellation | `SIGTERM`, then `SIGKILL` after a grace period — the same shape as the teacher sidecar's `stop()` |

What this buys, in the order it matters: cancellation that actually stops GPU spend; crash isolation, so one job's abort cannot take down a worker; a device set that can be chosen per job, which removes the §2 limitation entirely; a fresh allocator per job; and one execution model across both providers instead of two.

The teacher sidecar in `src/teacher/server.py` is already a working example of the whole pattern — spawn, health-probe, liveness-check at step boundaries, `SIGTERM` then `SIGKILL`, guaranteed teardown on exception. The subprocess trainer is that same machinery pointed at our own entrypoint, which is a large part of why this is worth doing rather than novel risk.

### Alternative considered: declare the local path dev-only

Refuse on-policy on `LocalGpuProvider` by configuration, document local as a development convenience, and leave the execution model alone. Zero code, and honest.

This is the right call **only if self-hosted training is never a product**. It does not fix §3.1–§3.3, which are correctness problems on the local path whether or not distillation exists. If a customer ever runs this on their own GPUs, all three ship with it.

## 5. Recommendation

**Do the subprocess trainer, as its own PR.** Not because on-policy needs it — on-policy already fails safely — but because cancellation that does not stop a GPU, and a crash that takes down a worker, are defects on their own terms, and "run it on your own hardware" is a plausible SKU for this platform.

Sequencing: merge the Stage 3 PR first, then branch. The subprocess change touches the shared path that **every** training job uses, so bundling it into a review-response commit would put an untested-on-GPU refactor of that path inside a merge decision that is otherwise about distillation.

Two things to settle before writing code:

1. **Measure whether heartbeats currently flush from the blocked loop** (§3.1). It decides whether this is a fix or a rescue.
2. **Decide whether Modal keeps calling the core directly.** It has no reason to spawn a child — it already gets a fresh process per job — so the child launch belongs in the local provider, not in the core. Keeping the core callable both ways preserves the current Modal path unchanged.

### Test plan the PR needs

Against a scripted fake child, in the shape `tests/test_teacher_server.py` already uses: child exits non-zero (error carries the code and the stderr tail), child never writes a result, child hangs past the activity timeout, cancellation sends `SIGTERM` then `SIGKILL`, the child's environment carries the intended device set, and the parent heartbeats while waiting. Plus one real end-to-end local run, since no fake proves the CUDA behaviour that motivated this.

### Out of scope

Sharing one teacher across concurrent runs, resuming across a teacher restart, and any change to the Modal path's reservation or recovery logic.

## 6. Unrelated defect found while reading this code

[gpu_provider.py](../apps/workers/src/gpu_provider.py) — `run_export_gguf`, the second of its three definitions in that file, is indented one level too deep, so it is nested inside `cancel_orphaned_gpu_calls` after that function's `return`. It is unreachable, and `LocalGpuProvider` therefore has no `run_export_gguf` at all: the call at `ExportGgufActivities` in [activities/export_gguf.py](../apps/workers/src/activities/export_gguf.py) raises `AttributeError` on the local provider. Introduced in `4a1bf57` (2026-07-24); GGUF export has never worked on the local path. Unrelated to distillation, and fixable on its own in one commit.
