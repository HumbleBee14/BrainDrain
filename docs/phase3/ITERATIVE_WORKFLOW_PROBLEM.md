# Iterative Training: Why the Current Architecture Is Wrong

> A deep dive into a real architectural bug in this codebase — and the Temporal principle it violates.

---

## First: A Quick Analogy

Imagine you're a manager at a factory. You have two types of people:

- **Workers** (activities) — people on the assembly line. Each one does one job: weld this part, paint that panel, test this circuit. If they mess up, you can ask them to redo *just their step*.
- **The Manager** (workflow) — you. You decide the order of operations, check quality between steps, and decide whether to continue or stop early. You don't weld anything yourself.

Now imagine you told one worker: "Hey, do the welding, THEN do the painting, THEN do the testing, THEN decide if quality is good enough, and if not, loop back and do it all again."

That worker is now doing everything. You (the manager) are sitting idle with no visibility. If the worker gets sick halfway through, *all* the work is lost — you don't even know which steps were completed.

**That's exactly what's happening in the iterative training code.**

---

## Temporal's Golden Rule

Temporal separates two responsibilities:

- **Activities** do a single unit of work — call an API, run one training round, download a file. They can fail, timeout, and be retried. They're the unit of execution. Think of them as **the hands**.

- **Workflows** orchestrate activities — they decide *what to do next* based on results. They survive crashes because Temporal replays the workflow's decision history to restore state. They never do compute themselves. Think of them as **the brain**.

The golden rule: **loops and decisions go in workflows. Compute goes in activities.**

This is the core reason Temporal exists. If you put loops inside activities, you're just writing normal Python with extra steps — you lose crash recovery, retry granularity, visibility, and decision-making. You've added Temporal's complexity without getting Temporal's benefits.

---

## What We Have Now (The Bug)

### The Current `TrainWorkflow`

```
TrainWorkflow
  └── execute_activity(start_training)    ← ONE activity, 6-hour timeout
        └── IterativeStrategy.execute()
              └── _train_iterative()
                    ├── for iteration in range(3):       ← loop INSIDE activity
                    │     ├── _train_sft(...)             ← 1-2 hours of GPU
                    │     ├── _evaluate_on_holdout(...)   ← validation check
                    │     └── _stream_metric(...)         ← "iteration complete"
                    └── return all_metrics
```

When a user chooses `mode="iterative"`, the workflow calls **one single** `start_training` activity. Inside that activity, `_train_iterative()` runs a Python `for` loop doing 3 separate SFT training rounds with evaluation after each.

The entire multi-round process — potentially 6+ hours of GPU compute — is one monolithic activity.

### The Actual Code

**`workflows/train.py`** — The workflow is almost empty. One activity call, nothing else:

```python
@workflow.defn
class TrainWorkflow:
    @workflow.run
    async def run(self, ...) -> StartTrainingOutput:
        result = await workflow.execute_activity(
            start_training,
            StartTrainingInput(...),
            task_queue="ml-pipeline-gpu",
            start_to_close_timeout=timedelta(hours=6),    # Everything must finish in 6 hours
            heartbeat_timeout=timedelta(minutes=5),
            retry_policy=workflow.RetryPolicy(maximum_attempts=1),  # No retries!
        )
        return result
```

**`activities/train_model.py`** — The activity contains the loop (the part that should be in the workflow):

```python
def _train_iterative(model, tokenizer, dataset, hp, ...):
    num_iterations = hp.get("num_iterations", 3)

    for iteration in range(num_iterations):       # This loop should be in the WORKFLOW
        iteration_metrics = _train_sft(...)       # 1-2 hrs GPU each
        eval_loss = _evaluate_on_holdout(...)     # Validation
        _stream_metric(job_id, {"event": "iteration_complete", ...})

    return all_metrics                            # No early stopping, just runs all 3
```

---

## Why This Is Broken (6 Specific Problems)

### 1. No Retry Granularity

**Scenario:** Iteration 3 fails (OOM, API error, hardware glitch) after iterations 1 and 2 each took 1.5 hours of GPU compute.

**What happens now:** Temporal retries the entire `start_training` activity from scratch. Iterations 1 and 2? Gone. That's 3 hours of GPU time wasted (potentially $50-200 in cloud costs).

**What should happen:** Temporal only retries iteration 3. Iterations 1 and 2 are already recorded as completed activities — Temporal skips them on replay.

### 2. No Workflow-Level Visibility

**Scenario:** A user starts iterative training and checks the Temporal UI to see progress.

**What they see now:** One activity blob called `start_training` running for hours. No idea which iteration they're on, what the eval losses look like, or how much longer it'll take.

**What they should see:** 6 separate activities in the Temporal timeline (3 train + 3 eval), each with its own duration, status, and result. Like this:

```
Temporal UI Timeline:
✅ train_sft_round (iteration 0)     — 1hr 23min — loss: 0.42
✅ evaluate_holdout (iteration 0)    — 8min — eval_loss: 0.38
✅ train_sft_round (iteration 1)     — 1hr 18min — loss: 0.35
✅ evaluate_holdout (iteration 1)    — 8min — eval_loss: 0.31
⏳ train_sft_round (iteration 2)     — running (47min elapsed)
○  evaluate_holdout (iteration 2)    — pending
```

### 3. No Early Stopping

**Scenario:** After iteration 2, the eval loss goes UP (0.31 → 0.45). The model is overfitting — more training is making it worse.

**What happens now:** The loop just keeps going. There's no early stopping logic at all. The `_train_iterative` function runs all `num_iterations` rounds regardless of results. Look at the code — it streams the eval_loss metric but never checks it:

```python
# Current code — streams metrics but never acts on them
_stream_metric(job_id, {
    "event": "iteration_complete",
    "eval_loss": eval_loss,       # ← logged but never checked!
})
# Just continues to next iteration...
```

**What should happen:** The workflow compares eval losses between iterations and breaks early if the model is getting worse. This decision belongs in the workflow, not the activity.

### 4. Timeout Risk

**The math:** 3 iterations × (1-2 hours SFT + evaluation) = 3-6+ hours. The activity has a 6-hour `start_to_close_timeout`. On a larger dataset or slower GPU, this will timeout and the entire job dies with no recovery.

**With separate activities:** Each SFT round gets its own 3-hour timeout (more than enough for one round). The total workflow can run for days if needed — Temporal doesn't timeout workflows, only activities.

### 5. No Crash Recovery

**Scenario:** The GPU worker crashes (OOM, hardware failure, deployment restart) between iteration 2 and 3.

**What happens now:** Temporal sees the `start_training` activity failed. With `maximum_attempts=1`, it doesn't even retry. If it did retry, it would restart from iteration 1.

**What should happen:** Temporal knows iterations 1 and 2 completed (they were separate activities). It replays the workflow, skips the completed activities, and resumes at iteration 3.

### 6. No Graceful Cancellation

**Scenario:** User sees bad metrics after iteration 1 and clicks "Cancel."

**What happens now:** Temporal cancels the `start_training` activity. The entire multi-round process is killed. Any partial progress from the current iteration is lost.

**What should happen:** The current iteration's activity finishes (or is cancelled), and the workflow stops looping. The completed iterations' checkpoints are preserved.

---

## This Bug Isn't Just in Iterative Mode

The same architectural problem affects the other multi-step strategies:

**`AlignedStrategy`** — runs `_train_sft()` THEN `_train_dpo()` inside one activity. If DPO fails after a 2-hour SFT round, the SFT work is lost.

**`ReasoningStrategy`** — runs `_train_sft()` THEN `_train_grpo()` inside one activity. Same problem.

These should also be split: SFT as one activity, DPO/GRPO as a second activity, with the workflow chaining them.

---

## How the Codebase Already Gets This Right

The irony is that other workflows in this same codebase follow the correct pattern perfectly.

### `IngestWorkflow` — Loop in the Workflow

```python
@workflow.defn
class IngestWorkflow:
    @workflow.run
    async def run(self, tenant_id, project_id, document_ids) -> dict:
        for doc_id in document_ids:                    # ← Loop in WORKFLOW ✅
            doc_info = await workflow.execute_activity( # ← Activity 1: fetch metadata
                get_document_info, doc_id,
                start_to_close_timeout=timedelta(seconds=30),
            )

            if doc_info.status == "parsed":            # ← Decision in WORKFLOW ✅
                continue                               #    Skip already-parsed docs

            await workflow.execute_activity(            # ← Activity 2: parse
                parse_document,
                ParseDocumentInput(...),
                start_to_close_timeout=timedelta(minutes=10),
                retry_policy=workflow.RetryPolicy(maximum_attempts=3),  # ← 3 retries per doc
            )
```

Notice the pattern:
- The **loop** is in the workflow (iterates over documents)
- The **decision** is in the workflow (skips already-parsed docs)
- Each document is processed by **separate activities** (independently retryable)
- If document 5 of 10 fails, documents 1-4 are done and Temporal won't redo them

This is exactly what iterative training should do — but with iterations instead of documents.

### `FullPipelineWorkflow` — Chaining Stages

```python
@workflow.defn
class FullPipelineWorkflow:
    @workflow.run
    async def run(self, ...) -> dict:
        ingest_result  = await execute_child_workflow(IngestWorkflow, ...)   # Stage 1
        refine_result  = await execute_child_workflow(RefineWorkflow, ...)   # Stage 2
        train_result   = await execute_child_workflow(TrainWorkflow, ...)    # Stage 3
        eval_result    = await execute_child_workflow(EvaluateWorkflow, ...) # Stage 4

        if training_config.get("auto_deploy", False):                       # Decision ✅
            deploy_result = await execute_activity(deploy_model, ...)       # Stage 5
```

Each stage is a child workflow — independently visible, retryable, and checkpointed. If training fails, ingestion and refinement are preserved. The `auto_deploy` decision is made by the workflow based on config.

**The iterative training loop should follow the exact same pattern.**

---

## What It Should Look Like

### Refactored Architecture

```
Before (broken):                         After (correct):

TrainWorkflow                            TrainIterativeWorkflow
  └── start_training (1 activity)          └── for i in range(3):          ← loop in WORKFLOW
        └── for loop (inside activity)           ├── train_sft_round       ← Activity 1
              ├── _train_sft()                   ├── evaluate_holdout      ← Activity 2
              ├── _evaluate_on_holdout()          └── if loss regressed:   ← DECISION in workflow
              └── no decisions                         break (early stop)
```

### The Two New Activities

**`train_sft_round`** — Does one thing: runs a single SFT training round.
- Loads model (or loads from previous iteration's checkpoint)
- Runs SFT with given hyperparams for one round
- Saves adapter checkpoint to S3
- Returns metrics (loss, steps, runtime) and the checkpoint path

**`evaluate_holdout`** — Does one thing: evaluates a model on validation data.
- Loads model + adapter from the checkpoint
- Runs evaluation on the held-out validation set
- Returns eval_loss

Each activity is focused, independently retryable, and has its own timeout.

### The New Workflow

```python
@workflow.defn
class TrainIterativeWorkflow:
    @workflow.run
    async def run(self, ...) -> StartTrainingOutput:
        previous_eval_loss = float("inf")
        best_adapter_path = None
        all_metrics = {}

        for iteration in range(num_iterations):
            # Activity 1: Train one SFT round (retryable, individually visible)
            train_result = await workflow.execute_activity(
                train_sft_round,
                TrainSftRoundInput(
                    iteration=iteration,
                    adapter_path=best_adapter_path,  # Continue from last checkpoint
                    ...
                ),
                task_queue="ml-pipeline-gpu",
                start_to_close_timeout=timedelta(hours=3),   # Per-round timeout
                heartbeat_timeout=timedelta(minutes=5),
                retry_policy=workflow.RetryPolicy(maximum_attempts=2),
            )

            # Activity 2: Evaluate on holdout (separate, fast)
            eval_result = await workflow.execute_activity(
                evaluate_holdout,
                EvaluateHoldoutInput(
                    adapter_path=train_result.adapter_path,
                    ...
                ),
                task_queue="ml-pipeline-gpu",
                start_to_close_timeout=timedelta(minutes=30),
            )

            all_metrics[f"iter_{iteration}"] = {
                **train_result.metrics,
                "eval_loss": eval_result.eval_loss,
            }

            # DECISION: Early stopping (in the workflow, where it belongs!)
            if eval_result.eval_loss > previous_eval_loss:
                workflow.logger.info(
                    "Eval loss regressed: %.4f -> %.4f. Stopping early at iteration %d.",
                    previous_eval_loss, eval_result.eval_loss, iteration,
                )
                break

            previous_eval_loss = eval_result.eval_loss
            best_adapter_path = train_result.adapter_path

        return StartTrainingOutput(
            adapter_path=best_adapter_path,
            metrics=all_metrics,
        )
```

### What This Gets You

| Capability | Before (activity loop) | After (workflow loop) |
|---|---|---|
| **Retry granularity** | Entire 3-round job retries from scratch | Only failed iteration retries |
| **Visibility** | 1 activity blob running for hours | 6 activities (3 train + 3 eval) in Temporal UI |
| **Early stopping** | Not implemented — runs all iterations blindly | Workflow stops when eval loss regresses |
| **Crash recovery** | Restart from iteration 1 | Resume from last completed iteration |
| **Timeout** | 6hr for entire job (risky) | 3hr per round (plenty of room) |
| **Cancellation** | Kill everything, lose all progress | Stop after current iteration, keep checkpoints |
| **GPU cost on failure** | Wasted $50-200 on retry | Only redo the failed ~$20 round |

---

## The Same Fix Should Apply to Aligned and Reasoning Modes

The `AlignedStrategy` (SFT → DPO) and `ReasoningStrategy` (SFT → GRPO) have the same problem — two distinct training phases crammed into one activity.

The fix follows the same principle:

```
AlignedWorkflow:
  ├── execute_activity(train_sft_round)     ← Activity 1: SFT phase
  └── execute_activity(train_dpo_round)     ← Activity 2: DPO phase (only if SFT succeeded)

ReasoningWorkflow:
  ├── execute_activity(train_sft_round)     ← Activity 1: SFT phase
  └── execute_activity(train_grpo_round)    ← Activity 2: GRPO phase (only if SFT succeeded)
```

If the DPO phase fails, the SFT work is preserved. The workflow can retry just the DPO phase.

---

## Key Takeaway

The principle is simple: **activities are the hands, workflows are the brain.**

- Hands do work (train one round, evaluate, download data)
- Brain makes decisions (should we stop early? should we retry? what's next?)

Right now, the iterative training mode puts both the hands AND the brain inside a single activity. The workflow is an empty shell that just says "go do everything." That defeats the entire purpose of using Temporal.

The fix is to extract the loop and the early-stopping decision into the workflow, leaving each SFT round and each evaluation as independent activities. This gives us retry granularity, crash recovery, visibility, and intelligent decision-making — exactly what Temporal was built for.

The codebase already demonstrates the correct pattern in `IngestWorkflow` (loop in workflow, activities per document) and `FullPipelineWorkflow` (child workflows per stage). The iterative training code just needs to follow the same architecture that's already working everywhere else.
