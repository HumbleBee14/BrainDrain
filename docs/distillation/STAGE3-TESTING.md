# Stage 3 — how to verify on-policy distillation

> Everything below is **manual and unrun**. Stage 3 is green on every automatic gate (see §7), but no GPU has executed an on-policy run, and the migration has not been applied to any database. This document is the honest list of what that leaves unproven, in the order it should be checked.

## 0. Apply migration 032 to a scratch database first

`032_training_job_parent_model.sql` has been reviewed but **never executed** — there was no local Postgres or running Docker available when it was written.

```bash
make infra                     # brings up Postgres, Redis, MinIO
createdb ekcron_scratch        # or psql -c 'CREATE DATABASE ekcron_scratch'
DATABASE_URL=postgres://…/ekcron_scratch cargo run -p platform-db --bin migrate
```

Check that the column, the partial index and the `ON DELETE SET NULL` behaviour all landed, and that a second run is a no-op (every statement is `IF NOT EXISTS`).

**`make migrate` uses `.env`, which points at the production database.** Never use it to try a new migration.

## 1. The question the topology decision could not answer

The plan gated Stage 3 on a spike comparing two topologies. One of them turned out to be excluded by arithmetic ([STAGE3-SPIKE-FINDINGS.md](STAGE3-SPIKE-FINDINGS.md) §2, and §0a.2 of the plan), so the architecture was settled without GPU time. What was *not* settled is the empirical part, and it is the first thing to measure:

**Does an on-policy pass improve parity over the Stage 2 baseline, and by how much per GPU-dollar?**

Run one improve pass on a model produced by Stage 2 on the same dataset, then compare the parity figures the model page now shows side by side. The threshold the plan set stands: **under a 1-point gain, revisit the approach rather than tuning it.** A custom trainer (option C) only becomes justified if this measurement disappoints.

## 2. The first real run

Costs real money — two 80GB-class GPUs for the duration, quoted on the card before you click.

1. Take a distilled model whose teacher is in the hosted catalog. Its page should show **Sharpen against the teacher** with a dollar estimate.
2. Click **Improve**. Expect a new training job in `distill` mode on the `a10080gb_dual` class.
3. In the container logs, confirm in order: the teacher's `trl vllm-serve` process starting on GPU 0, `/health/` answering, then the trainer beginning steps on GPU 1.
4. When it finishes, the new model's page should show **Sharpening result** with both parity figures.

Watch for these specifically, since each is a place where the code is doing something it has never done:

- **The device split.** `CUDA_VISIBLE_DEVICES` must isolate the two, and it is only read the first time a process touches CUDA — so the reservation happens in `_reserve_student_devices`, before the engine loads anything, and a process that cannot be confined refuses the run rather than sharing a card. Check `nvidia-smi` mid-run: the teacher and the trainer must appear on different devices. Both on one is an out-of-memory kill waiting for the first long sequence.
- **Teacher boot time.** The 30-minute startup budget was chosen for a cold weight cache pulling tens of gigabytes. Time it; if a warm cache is much faster, `teacher_startup_timeout_secs` can come down.
- **Rollout throughput.** This is the number the quote guesses at (`APP_ON_POLICY_TOKENS_PER_SEC`, default 40). Measure the real figure and set it per deployment — the default is deliberately pessimistic, so quotes should come in high at first.

## 3. Failure paths worth provoking

Each of these is unit-tested against a fake process; none has been seen against a real one.

| Provoke | Expected |
| --- | --- |
| Ask for a teacher too large for its card | The run fails at boot with the teacher's exit code and the model name, not a 30-minute timeout |
| `kill` the teacher mid-training | The next step boundary fails the job non-retryably. **A run must never finish successfully after its teacher dies** — that is the one silent-corruption path here |
| Cancel the workflow mid-run | Container is reclaimed; teacher dies with it. Verify no GPU is still billing afterwards |
| Run on the Unsloth image | Refused immediately with "requires the on-policy image", before any process is spawned |
| Request `fp8`/`int4` for a served teacher | Refused as unsupported rather than silently served at bf16 |

## 4. Cost and caps

- Both the quote and the charge should use the paired class. A job quoted at a single-card rate and billed at a double one is the bug to look for.
- A finished on-policy job writes **two** billing rows, not one: `training` for the student's share and `teacher_serving` for the teacher's, split by device count. Confirm they re-add to the container's cost — one row at the full amount means the tenant is billed twice, and no `teacher_serving` row means the spend cap cannot see the run at all.
- The teacher-GPU spend cap sums both operations. Set `APP_TEACHER_GPU_SPEND_CAP_STARTER` low, run one improve pass to completion, and confirm the next admission refuses with the spend-cap message — the cap is what proves the `teacher_serving` row was delivered and not just written.
- A failed run splits the same way. Kill a run after the teacher has booted and confirm the cap still grew.
- The job should train the epoch count it was quoted for. `num_train_epochs` in the job's hyperparams, the `epochs` in its `teacher.extraction` block, and the trainer's own reported epochs must be the same number.
- `parent_model_id` should be set on the new job and NULL on every other job in the table.

## 5. Configuration

| Setting | Default | Why it is configuration |
| --- | --- | --- |
| `APP_ON_POLICY_TOKENS_PER_SEC` | 40 | The one unmeasured number in the quote — see §2 |
| `APP_MODAL_ON_POLICY_FUNCTION_NAME` | `train_on_policy` | Deployed function name for the vLLM-capable image |
| `rollout_temperature` (hyperparam) | 1.0 | Grading text the student would not have written defeats being on-policy |
| `on_policy_lambda` (hyperparam) | 1.0 | Fraction of on-policy data; below 1.0 mixes in teacher-written batches |
| `distill_beta` (hyperparam) | 1.0 | 1.0 is reverse KL; 0 would repeat what the off-policy stages already do, and is refused |
| `distill_objective` (hyperparam) | `jsd` | `iw_opd` is reachable but unproven — the parity numbers should promote it, not a guess |
| `use_vllm_rollouts` (hyperparam) | `false` | HF generate is slower but cannot lose an out-of-memory fight with the student. First optimization to measure after a successful run |

## 6. Known limitations, stated rather than fixed

- **One teacher per run.** Concurrent improve passes each boot their own teacher, on their own reserved port; nothing is shared. Fixing that means reintroducing the lifecycle machinery §0a.3 of the plan deliberately removed.
- **On-policy needs a worker that has not run other GPU work.** The student is confined to its card by `CUDA_VISIBLE_DEVICES`, which a process can only act on before it first touches CUDA. Modal gives every job a fresh container, so the documented path is unaffected. On `LocalGpuProvider`, where training runs in the long-lived worker process, a worker that has already trained something refuses the run instead of putting the student on the teacher's card. Fixing it properly means running the trainer in a subprocess.
- **No calibration metric.** ECE stays unimplemented because its definition for generative models is ambiguous and the plan gated it on a design spike that has not happened. Shipping a number would be inventing one.
- **Cost is billed as one container.** The teacher's share is separated for the spend cap by device count, which is exact for a homogeneous container but would not be if mixed-GPU classes were ever offered.
- **The reverse-KL signal is top-1 plus a tail bucket.** Not a shortcut — a top-k endpoint cannot answer anything richer, and a hand-written loss would face the same wall ([STAGE3-SPIKE-FINDINGS.md](STAGE3-SPIKE-FINDINGS.md) §7).
- **No resume across teacher restarts.** The plan imagined a trainer that checkpoint-resumes after a teacher restart. It fails loudly instead: a bounded retry against a teacher whose weights may have reloaded differently is a worse guarantee than a clean failure.

## 7. What the automatic gates do cover

```bash
cargo test --workspace                     # 586 tests
cargo clippy --workspace --all-targets -- -D warnings
cd apps/workers && python -m pytest tests/ # 759 tests
cd apps/web && pnpm type-check && pnpm lint
```

Notably covered without a GPU: the teacher lifecycle against a scripted fake process (ready, dies at boot, dies mid-run, ignores SIGTERM, missing CLI, a port answered by someone else), every configuration rule TRL enforces inside the trainer, the device split and the order it must happen in relative to loading the student, the platform-owned hyperparam guard, tenant scoping of the parent model, that the epoch count priced is the epoch count trained, that the billing split re-adds to one container's cost, and that a multi-device class is priced and mapped consistently across Rust and Python.

Two of those exist because the gates did not catch what a review did: nothing asserted that the GPU reservation happened before the model loaded, and nothing checked that an operation the spend cap sums is one a worker actually writes. Both are now tests rather than intentions.
