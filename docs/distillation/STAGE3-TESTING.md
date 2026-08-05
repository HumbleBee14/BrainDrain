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

**`make migrate` uses `.env`, which points at the production Neon database.** Never use it to try a new migration.

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

- **The device split.** `CUDA_VISIBLE_DEVICES` must isolate the two. If both land on one card, one of them dies out of memory — the guard in `split_devices` only catches a container that came up with a single GPU, not a mis-set variable.
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
- The teacher-GPU spend cap covers on-policy as well as extraction. Set `APP_TEACHER_GPU_SPEND_CAP_STARTER` low and confirm admission refuses with the spend-cap message.
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

- **One teacher per run.** Concurrent improve passes each boot their own teacher; nothing is shared. Fixing that means reintroducing the lifecycle machinery §0a.3 of the plan deliberately removed.
- **No calibration metric.** ECE stays unimplemented because its definition for generative models is ambiguous and the plan gated it on a design spike that has not happened. Shipping a number would be inventing one.
- **Cost is billed as one container.** The teacher's share is separated for the spend cap by device count, which is exact for a homogeneous container but would not be if mixed-GPU classes were ever offered.
- **The reverse-KL signal is top-1 plus a tail bucket.** Not a shortcut — a top-k endpoint cannot answer anything richer, and a hand-written loss would face the same wall ([STAGE3-SPIKE-FINDINGS.md](STAGE3-SPIKE-FINDINGS.md) §7).
- **No resume across teacher restarts.** The plan imagined a trainer that checkpoint-resumes after a teacher restart. It fails loudly instead: a bounded retry against a teacher whose weights may have reloaded differently is a worse guarantee than a clean failure.

## 7. What the automatic gates do cover

```bash
cargo test --workspace                     # 577 tests
cargo clippy --workspace --all-targets -- -D warnings
cd apps/workers && python -m pytest tests/ # 735 tests
cd apps/web && pnpm type-check && pnpm lint
```

Notably covered without a GPU: the teacher lifecycle against a scripted fake process (ready, dies at boot, dies mid-run, ignores SIGTERM, missing CLI), every configuration rule TRL enforces inside the trainer, the device split, the platform-owned hyperparam guard, tenant scoping of the parent model, and that a multi-device class is priced and mapped consistently across Rust and Python.
