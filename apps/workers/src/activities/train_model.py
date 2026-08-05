"""Training activities — runs fine-tuning jobs via pluggable strategies.

Strategy-based modes (dispatched by start_training activity):
  - quick:     SFT only (fastest iteration)
  - aligned:   SFT → DPO (production quality alignment)
  - reasoning: SFT → GRPO (reward-guided reasoning optimization)
  - distill:   SFT on teacher-written data, or KL against the teacher's stored
               per-token distributions when the job's distill_method is 'logit'

Workflow-based activities (called by TrainIterativeWorkflow):
  - train_sft_round:    Single SFT iteration with checkpoint save
  - evaluate_holdout:   Validation eval for early stopping decisions

Uses TrainingEngine protocol (default: Unsloth) for model loading,
LLMJudge protocol for scoring, and Redis streams for real-time metrics.
"""

import asyncio
import json
import logging
import math
import os
import tempfile
import time
import uuid
from datetime import UTC, datetime
from pathlib import Path

from temporalio import activity
from temporalio.exceptions import ApplicationError

from src import s3_paths
from src.activities.distill_loss import (
    DistillLossConfig,
    collate_distill_batch,
    distillation_loss,
)
from src.activities.llm_judge import OpenAICompatibleJudge
from src.activities.stubs import (
    EvaluateHoldoutInput,
    EvaluateHoldoutOutput,
    FinalizeIterativeTrainingInput,
    StartTrainingInput,
    StartTrainingOutput,
    TrainSftRoundInput,
    TrainSftRoundOutput,
)
from src.activities.training_engine import (
    get_engine,
    get_strategy,
    register_strategy,
)
from src.backends.judge import get as get_judge
from src.constants import (
    GPU_DEFAULT_DEVICE_COUNT,
    GPU_DEFAULT_HOURLY_RATE,
    GPU_DEVICE_COUNTS,
    GPU_HOURLY_RATES,
    LOGIT_DISTILL_METHOD,
    ON_POLICY_DISTILL_METHOD,
    TEXT_DISTILL_METHOD,
    TrainingJobStatus,
)
from src.gpu_provider import GpuProvider
from src.heartbeat import safe_heartbeat
from src.infra import InfraContainer
from src.notifications import EVENT_TRAINING_COMPLETE, enqueue_notification
from src.teacher.messages import TOKENIZER_MISMATCH_MESSAGE as _TEACHER_ARTIFACT_MISMATCH
from src.tenant_config import TenantLlmConfig

logger = logging.getLogger("platform.training")

_JUDGE_BACKED_MODES = frozenset({"aligned", "reasoning"})

# Warmup beyond this fraction of a run leaves the LR ramping for most of it.
_MAX_WARMUP_FRACTION = 0.1

_DISTILL_MODE = "distill"
_DEFAULT_DISTILL_METHOD = TEXT_DISTILL_METHOD

# The public TrainingMode stays `distill`; how much of the teacher a run copies is
# an orthogonal axis, so the registry key is composite and no internal strategy
# name can ever leak into the API enum.
_DISTILL_STRATEGY_KEYS = {
    TEXT_DISTILL_METHOD: "distill",
    LOGIT_DISTILL_METHOD: "distill_logit",
    ON_POLICY_DISTILL_METHOD: "distill_on_policy",
}

_TEACHER_ARTIFACTS_HYPERPARAM = "teacher_artifacts_prefix"
DISTILL_METHOD_HYPERPARAM = "distill_method"

# vLLM's `--dtype` takes no quantized values: fp8 and int4 are separate
# quantization flags with their own weight formats. Mapping them onto bfloat16
# would run a full-precision teacher while reporting a quantized one, and the
# teacher's distribution is the entire product of this pass.
_TEACHER_DTYPE_BY_PRECISION = {"bf16": "bfloat16"}

# A teacher this size is minutes of weight loading even from a warm cache, and a
# cold cache pulls tens of gigabytes first.
_DEFAULT_TEACHER_STARTUP_TIMEOUT_SECS = 1800


def resolve_strategy_key(mode: str, distill_method: str | None) -> str:
    """Registry key for a (mode, distill_method) pair.

    Modes other than `distill` have no fidelity axis, and a method asked for
    anyway is an error rather than something to ignore: running plain SFT for a
    job that already paid a teacher to score its data would report success for
    training that never happened.
    """
    method = distill_method or _DEFAULT_DISTILL_METHOD
    if mode != _DISTILL_MODE:
        if method != _DEFAULT_DISTILL_METHOD:
            raise ValueError(f"distill_method '{method}' has no meaning for mode '{mode}'")
        return mode

    key = _DISTILL_STRATEGY_KEYS.get(method)
    if key is None:
        available = ", ".join(sorted(_DISTILL_STRATEGY_KEYS))
        raise ValueError(f"Unknown distill_method: '{method}'. Available: {available}")
    return key


class StartTrainingActivity:
    def __init__(self, infra: InfraContainer, gpu_provider: GpuProvider | None = None):
        self.infra = infra
        self.gpu_provider = gpu_provider

    @staticmethod
    async def _existing_result_if_completed(db, job_id: str) -> StartTrainingOutput | None:
        """Reconstruct the output of an already-completed job, or None.

        Used to make the activity idempotent: a Temporal retry after a successful
        first attempt returns the persisted result instead of re-running training
        or inserting a duplicate model row.
        """
        row = await db.fetchrow(
            """SELECT j.status, j.metrics, m.id AS model_id, m.adapter_path,
                m.adapter_size_bytes
            FROM training_jobs j
            LEFT JOIN models m ON m.training_job_id = j.id
            WHERE j.id = $1""",
            job_id,
        )
        if row is None or row["status"] != TrainingJobStatus.COMPLETED:
            return None
        if row["adapter_path"] is None:
            return None
        metrics = row["metrics"]
        if isinstance(metrics, str):
            metrics = json.loads(metrics)
        return StartTrainingOutput(
            adapter_path=row["adapter_path"],
            adapter_size_bytes=row["adapter_size_bytes"] or 0,
            metrics=metrics or {},
            model_id=str(row["model_id"] or ""),
        )

    @activity.defn(name="start_training")
    async def run(self, input: StartTrainingInput) -> StartTrainingOutput:
        """Run a fine-tuning job. Called by TrainWorkflow.

        Dispatches to the configured GpuProvider (local or Modal).
        Falls back to direct local execution if no provider is set.
        """
        db = self.infra.db
        job_id = input.training_job_id

        try:
            # Claim the job for training only if it is still runnable. 'training'
            # is included so a retry after a crash mid-run (start_training has
            # maximum_attempts=2, and the first attempt already moved the job to
            # 'training' before dying) legitimately re-runs. A job that finished
            # (completed) or was cancelled/failed is NOT re-run: it falls out of
            # this set and is handled below. Double-billing from a rare zombie
            # double-run is prevented by the status-guarded completion UPDATE,
            # not by excluding 'training' here.
            started_id = await db.fetchval(
                """UPDATE training_jobs SET status = $1, started_at = NOW()
                WHERE id = $2 AND status IN ('pending', 'provisioning', 'training')
                RETURNING id""",
                TrainingJobStatus.TRAINING,
                job_id,
            )
            if started_id is None:
                existing = await self._existing_result_if_completed(db, job_id)
                if existing is not None:
                    logger.info(
                        "Job %s already completed; returning existing result "
                        "(idempotent activity retry)",
                        job_id,
                    )
                    return existing
                # Cancelled/failed before we could start — do not run training.
                raise ApplicationError(
                    f"Training job {job_id} is no longer runnable",
                    non_retryable=True,
                )

            from dataclasses import asdict

            from src.tenant_config import get_tenant_llm_config

            llm_config = await get_tenant_llm_config(
                db=self.infra.db,
                tenant_id=input.tenant_id,
                default_api_base_url=self.infra.settings.llm_api_base_url,
                default_api_key=self.infra.settings.llm_api_key,
                default_model=self.infra.settings.llm_model,
                encryption_key=self.infra.settings.settings_encryption_key,
                settings=self.infra.settings,
            )

            if input.mode in _JUDGE_BACKED_MODES:
                await asyncio.to_thread(
                    get_judge(
                        self.infra.settings.judge_backend,
                        api_base=llm_config.api_base_url,
                        api_key=llm_config.api_key,
                        model=llm_config.model,
                        max_retries=self.infra.settings.judge_max_retries,
                        on_failure=self.infra.settings.judge_on_failure,
                    ).preflight
                )

            if self.gpu_provider is not None:
                result_dict = await self.gpu_provider.run_training(
                    tenant_id=input.tenant_id,
                    training_job_id=job_id,
                    dataset_path=input.dataset_path,
                    base_model=input.base_model,
                    method=input.method,
                    mode=input.mode,
                    hyperparams=input.hyperparams,
                    gpu_class=input.gpu_class,
                    llm_config=asdict(llm_config),
                )
                result = StartTrainingOutput(
                    adapter_path=result_dict["adapter_path"],
                    adapter_size_bytes=result_dict["adapter_size_bytes"],
                    metrics=result_dict["metrics"],
                )
            else:
                result = await _run_training(input, self.infra)

            actual_cost = float(result.metrics.get("estimated_cost") or 0.0)
            gpu_seconds = _extract_training_runtime_seconds(result.metrics)
            model_name = f"{input.base_model.split('/')[-1]}-{input.mode}-{job_id[:8]}"

            async with db.acquire() as conn:
                async with conn.transaction():
                    # Guard the terminal transition: only a job still in a runnable
                    # state may be completed. If it was cancelled or reaped while this
                    # activity was running (Temporal termination does not preempt an
                    # executing activity), the UPDATE matches 0 rows and we must NOT
                    # insert a model / enqueue billing / notify — doing so would
                    # resurrect a cancelled job and double-bill the tenant.
                    completed_id = await conn.fetchval(
                        """UPDATE training_jobs
                        SET status = $1,
                            metrics = $3,
                            actual_cost = $4,
                            completed_at = NOW()
                        WHERE id = $2 AND status IN ('training', 'provisioning')
                        RETURNING id""",
                        TrainingJobStatus.COMPLETED,
                        job_id,
                        json.dumps(result.metrics),
                        actual_cost,
                    )

                    if completed_id is None:
                        logger.warning(
                            "Job %s not in a runnable state at completion; "
                            "skipping model insert, billing and notification",
                            job_id,
                        )
                        return result

                    project_id = await _get_project_id(conn, job_id)

                    # Auto-increment version for the same base_model within this project
                    max_version = await conn.fetchval(
                        """SELECT COALESCE(MAX(version), 0) FROM models
                        WHERE project_id = $1 AND tenant_id = $2 AND base_model = $3""",
                        project_id,
                        input.tenant_id,
                        input.base_model,
                    )
                    next_version = (max_version or 0) + 1

                    created_model_id = await conn.fetchval(
                        """INSERT INTO models
                        (tenant_id, project_id, training_job_id, name, base_model,
                         adapter_path, adapter_size_bytes, version)
                        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                        RETURNING id""",
                        input.tenant_id,
                        project_id,
                        job_id,
                        model_name,
                        input.base_model,
                        result.adapter_path,
                        result.adapter_size_bytes,
                        next_version,
                    )
                    result.model_id = str(created_model_id)

                    await _append_training_billing_outbox(
                        conn,
                        tenant_id=input.tenant_id,
                        job_id=job_id,
                        outcome="completed",
                        gpu_seconds=gpu_seconds,
                        cost_usd=actual_cost,
                        teacher_share=teacher_serving_share(input.gpu_class, input.hyperparams),
                        metadata={
                            "status": "completed",
                            "mode": input.mode,
                            "method": input.method,
                            "base_model": input.base_model,
                            "gpu_class": input.gpu_class,
                        },
                    )

                    await enqueue_notification(
                        conn,
                        tenant_id=input.tenant_id,
                        event_type=EVENT_TRAINING_COMPLETE,
                        payload={
                            "status": "completed",
                            "training_job_id": job_id,
                            "model_name": model_name,
                            "base_model": input.base_model,
                            "subject": f"Training complete: {model_name}",
                            "message": (
                                f"Fine-tuning of {input.base_model} finished. "
                                f"Model '{model_name}' is ready to evaluate and deploy."
                            ),
                        },
                    )

            logger.info("Training completed for job %s, model: %s", job_id, model_name)
            return result

        except Exception as e:
            logger.exception("Training failed for job %s", job_id)
            async with db.acquire() as conn:
                async with conn.transaction():
                    # Only mark FAILED (and bill the failed run) if the job was
                    # actually running. A job cancelled/reaped concurrently must not
                    # be flipped to failed or billed a second time.
                    failed_id = await conn.fetchval(
                        """UPDATE training_jobs
                        SET status = $1, error_message = $3, completed_at = NOW()
                        WHERE id = $2 AND status IN ('training', 'provisioning', 'pending')
                        RETURNING id""",
                        TrainingJobStatus.FAILED,
                        job_id,
                        str(e)[:2000],
                    )

                    if failed_id is None:
                        logger.warning(
                            "Job %s not in a runnable state at failure; "
                            "skipping failed-billing and notification",
                            job_id,
                        )
                        raise

                    await _finalize_failed_training_billing(
                        conn,
                        job_id,
                        self.infra.settings,
                        mode=input.mode,
                        method=input.method,
                        base_model=input.base_model,
                        hyperparams=input.hyperparams,
                    )

                    await enqueue_notification(
                        conn,
                        tenant_id=input.tenant_id,
                        event_type=EVENT_TRAINING_COMPLETE,
                        payload={
                            "status": "failed",
                            "training_job_id": job_id,
                            "base_model": input.base_model,
                            "subject": "Training job failed",
                            "message": (
                                f"Fine-tuning of {input.base_model} failed: {str(e)[:500]}"
                            ),
                        },
                    )

            raise


class TrainSftRoundActivity:
    """Single SFT iteration for the iterative workflow.

    Each round: load model (+adapter if continuing), train one SFT pass,
    save adapter checkpoint to S3. The loop lives in TrainIterativeWorkflow.
    """

    def __init__(self, infra: InfraContainer, gpu_provider: GpuProvider | None = None):
        self.infra = infra
        self.gpu_provider = gpu_provider

    @activity.defn(name="train_sft_round")
    async def run(self, input: TrainSftRoundInput) -> TrainSftRoundOutput:
        job_id = input.training_job_id

        # Mark training as started on the first iteration (DB — always worker-side)
        if input.iteration == 0:
            await self.infra.db.execute(
                "UPDATE training_jobs SET status = $1, started_at = NOW() WHERE id = $2",
                TrainingJobStatus.TRAINING,
                job_id,
            )

        # Dispatch the GPU work to the configured provider (local or Modal).
        # Falls back to in-process execution when no provider is set.
        if self.gpu_provider is not None:
            result_dict = await self.gpu_provider.run_sft_round(
                tenant_id=input.tenant_id,
                training_job_id=job_id,
                dataset_path=input.dataset_path,
                base_model=input.base_model,
                method=input.method,
                hyperparams=input.hyperparams,
                iteration=input.iteration,
                adapter_path=input.adapter_path,
                gpu_class=input.gpu_class,
            )
            return TrainSftRoundOutput(
                adapter_path=result_dict["adapter_path"],
                adapter_size_bytes=result_dict["adapter_size_bytes"],
                metrics=result_dict["metrics"],
            )

        return await run_sft_round_core(
            input,
            s3=self.infra.s3,
            s3_bucket=self.infra.s3_bucket,
            settings=self.infra.settings,
        )


class EvaluateHoldoutActivity:
    """Run holdout validation after an SFT round.

    Loads the adapter from the iteration checkpoint and evaluates
    on the validation split. Returns eval_loss for early stopping decisions.
    Streams progress metrics to Redis for real-time UI visibility.
    """

    def __init__(self, infra: InfraContainer, gpu_provider: GpuProvider | None = None):
        self.infra = infra
        self.gpu_provider = gpu_provider

    @activity.defn(name="evaluate_holdout")
    async def run(self, input: EvaluateHoldoutInput) -> EvaluateHoldoutOutput:
        # Dispatch the GPU work to the configured provider (local or Modal).
        # Falls back to in-process execution when no provider is set.
        if self.gpu_provider is not None:
            result_dict = await self.gpu_provider.run_evaluate_holdout(
                tenant_id=input.tenant_id,
                training_job_id=input.training_job_id,
                adapter_path=input.adapter_path,
                base_model=input.base_model,
                method=input.method,
                dataset_path=input.dataset_path,
                hyperparams=input.hyperparams,
                iteration=input.iteration,
                gpu_class=input.gpu_class,
            )
            return EvaluateHoldoutOutput(
                eval_loss=result_dict["eval_loss"],
                metrics=result_dict["metrics"],
            )

        return await run_evaluate_holdout_core(
            input,
            s3=self.infra.s3,
            s3_bucket=self.infra.s3_bucket,
            settings=self.infra.settings,
        )


class FinalizeIterativeTrainingActivity:
    """DB lifecycle for iterative training completion.

    Updates training job status, calculates actual cost from per-iteration
    runtimes, and creates the model record. Mirrors the post-training logic
    in StartTrainingActivity but for the iterative workflow path.
    """

    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="finalize_iterative_training")
    async def run(self, input: FinalizeIterativeTrainingInput) -> str:
        db = self.infra.db
        job_id = input.training_job_id

        # Calculate actual cost from aggregate iteration runtimes
        gpu_rate = GPU_HOURLY_RATES.get(input.gpu_class or "", GPU_DEFAULT_HOURLY_RATE)
        total_runtime = _sum_gpu_runtime_seconds(input.metrics)
        runtime_hours = total_runtime / 3600.0
        actual_cost = round(runtime_hours * gpu_rate, 2)
        input.metrics["estimated_cost"] = actual_cost

        model_name = f"{input.base_model.split('/')[-1]}-{input.mode}-{job_id[:8]}"
        gpu_seconds = int(round(total_runtime))

        async with db.acquire() as conn:
            async with conn.transaction():
                await conn.execute(
                    """UPDATE training_jobs
                    SET status = $1,
                        metrics = $3,
                        actual_cost = $4,
                        completed_at = NOW()
                    WHERE id = $2""",
                    TrainingJobStatus.COMPLETED,
                    job_id,
                    json.dumps(input.metrics),
                    actual_cost,
                )

                project_id = await _get_project_id(conn, job_id)

                max_version = await conn.fetchval(
                    """SELECT COALESCE(MAX(version), 0) FROM models
                    WHERE project_id = $1 AND tenant_id = $2 AND base_model = $3""",
                    project_id,
                    input.tenant_id,
                    input.base_model,
                )
                next_version = (max_version or 0) + 1

                created_model_id = await conn.fetchval(
                    """INSERT INTO models
                    (tenant_id, project_id, training_job_id, name, base_model,
                     adapter_path, adapter_size_bytes, version)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                    RETURNING id""",
                    input.tenant_id,
                    project_id,
                    job_id,
                    model_name,
                    input.base_model,
                    input.adapter_path,
                    input.adapter_size_bytes,
                    next_version,
                )

                await _append_training_billing_outbox(
                    conn,
                    tenant_id=input.tenant_id,
                    job_id=job_id,
                    outcome="completed",
                    gpu_seconds=gpu_seconds,
                    cost_usd=actual_cost,
                    metadata={
                        "status": "completed",
                        "mode": input.mode,
                        "method": "qlora",
                        "base_model": input.base_model,
                        "gpu_class": input.gpu_class,
                        "iterative": True,
                    },
                )

                await enqueue_notification(
                    conn,
                    tenant_id=input.tenant_id,
                    event_type=EVENT_TRAINING_COMPLETE,
                    payload={
                        "status": "completed",
                        "training_job_id": job_id,
                        "model_name": model_name,
                        "base_model": input.base_model,
                        "subject": f"Training complete: {model_name}",
                        "message": (
                            f"Fine-tuning of {input.base_model} finished. "
                            f"Model '{model_name}' is ready to evaluate and deploy."
                        ),
                    },
                )

        logger.info(
            "Iterative training finalized for job %s, model: %s (cost: $%.2f)",
            job_id,
            model_name,
            actual_cost,
        )
        return str(created_model_id)


def _download_adapter(s3_prefix: str, local_dir: Path, s3, bucket: str):
    """Download all files under an S3 prefix to a local directory."""
    response = s3.list_objects_v2(Bucket=bucket, Prefix=s3_prefix)
    for obj in response.get("Contents", []):
        key = obj["Key"]
        relative = key[len(s3_prefix) :]
        if not relative:
            continue
        local_file = local_dir / relative
        local_file.parent.mkdir(parents=True, exist_ok=True)
        s3.download_file(bucket, key, str(local_file))
    logger.info("Downloaded adapter from %s to %s", s3_prefix, local_dir)


async def run_training_core(
    input: StartTrainingInput,
    *,
    s3,
    s3_bucket: str,
    settings,
    llm_config: TenantLlmConfig,
) -> StartTrainingOutput:
    """Pure-compute training core — needs only S3 + a resolved llm_config.

    No Postgres, no Redis. Runs identically in-process (LocalGpuProvider) or
    inside a remote Modal GPU container.
    """
    hp = input.hyperparams
    job_id = input.training_job_id

    # The strategy is resolved before the engine because a strategy can require
    # one: on-policy distillation cannot run under Unsloth, whose image is unable
    # to carry the vLLM its teacher needs.
    strategy = get_strategy(resolve_strategy_key(input.mode, hp.get(DISTILL_METHOD_HYPERPARAM)))
    engine = get_engine(settings, required=getattr(strategy, "required_engine", None))

    # Before the engine loads a single weight: a strategy that runs a teacher in
    # this container has to claim its cards while CUDA_VISIBLE_DEVICES still means
    # something.
    teacher_devices = _reserve_student_devices(strategy)

    _get_metrics_collector(settings)

    with tempfile.TemporaryDirectory(prefix=f"train-{job_id[:8]}-") as tmpdir:
        tmpdir_path = Path(tmpdir)
        dataset_local = tmpdir_path / "dataset.jsonl"
        _download_dataset(input.dataset_path, dataset_local, s3, s3_bucket)

        dataset = _load_chatml_dataset(dataset_local)
        logger.info("Loaded dataset: %d examples", len(dataset))

        load_in_4bit = input.method == "qlora"
        max_seq_length = hp.get("max_seq_length", 2048)
        model, tokenizer = engine.load_model(
            model_name=input.base_model,
            max_seq_length=max_seq_length,
            load_in_4bit=load_in_4bit,
        )
        target_modules = hp.get(
            "target_modules",
            ["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
        )
        model = engine.attach_adapter(
            model,
            r=hp.get("r", 16),
            lora_alpha=hp.get("lora_alpha", 16),
            lora_dropout=hp.get("lora_dropout", 0),
            target_modules=target_modules,
        )

        metrics = strategy.execute(
            model=model,
            tokenizer=tokenizer,
            dataset=dataset,
            hp=hp,
            job_id=job_id,
            max_seq_length=max_seq_length,
            tenant_id=input.tenant_id,
            base_model=input.base_model,
            dataset_path=input.dataset_path,
            s3=s3,
            bucket=s3_bucket,
            llm_config=llm_config,
            settings=settings,
            teacher_devices=teacher_devices,
        )

        gpu_rate = GPU_HOURLY_RATES.get(input.gpu_class or "", GPU_DEFAULT_HOURLY_RATE)
        total_runtime = _sum_gpu_runtime_seconds(metrics)
        metrics["estimated_cost"] = round((total_runtime / 3600.0) * gpu_rate, 2)

        adapter_dir = tmpdir_path / "adapter"
        engine.save_adapter(model, tokenizer, adapter_dir)
        adapter_s3_path = s3_paths.adapter_training_prefix(input.tenant_id, job_id)
        adapter_size = _upload_adapter(adapter_dir, adapter_s3_path, s3, s3_bucket)

        return StartTrainingOutput(
            adapter_path=adapter_s3_path,
            adapter_size_bytes=adapter_size,
            metrics=metrics,
        )


async def run_sft_round_core(
    input: TrainSftRoundInput,
    *,
    s3,
    s3_bucket: str,
    settings,
) -> TrainSftRoundOutput:
    """Pure-compute SFT round — needs only S3. No Postgres, no Redis.

    One iteration of the iterative workflow: load model (+ prior adapter if
    continuing), train one SFT pass, save the adapter checkpoint to S3.
    Runs identically in-process (LocalGpuProvider) or inside a remote Modal
    GPU container. The `iteration == 0` status write stays worker-side.
    """
    engine = get_engine(settings)
    hp = input.hyperparams
    job_id = input.training_job_id
    iteration = input.iteration

    _get_metrics_collector(settings)

    with tempfile.TemporaryDirectory(prefix=f"sft-round-{job_id[:8]}-") as tmpdir:
        tmpdir_path = Path(tmpdir)

        dataset_local = tmpdir_path / "dataset.jsonl"
        _download_dataset(input.dataset_path, dataset_local, s3, s3_bucket)
        dataset = _load_chatml_dataset(dataset_local)

        load_in_4bit = input.method == "qlora"
        max_seq_length = hp.get("max_seq_length", 2048)
        model, tokenizer = engine.load_model(
            model_name=input.base_model,
            max_seq_length=max_seq_length,
            load_in_4bit=load_in_4bit,
        )

        if input.adapter_path:
            # Resuming from a previous iteration: load the saved adapter directly
            # with PeftModel.from_pretrained(is_trainable=True) so its own config +
            # weights are restored exactly and remain trainable for this round. Do
            # NOT call attach_adapter here — the loaded PEFT model already carries
            # the adapter. The old attach_adapter() + load_adapter("default")
            # pattern created a fresh random "default" adapter and then failed to
            # load the saved weights into it (state_dict mismatch); this is the
            # same broken pattern run_evaluate_holdout_core documents and avoids.
            prev_adapter_dir = tmpdir_path / "prev_adapter"
            prev_adapter_dir.mkdir(parents=True)
            _download_adapter(input.adapter_path, prev_adapter_dir, s3, s3_bucket)

            from peft import PeftModel

            model = PeftModel.from_pretrained(model, str(prev_adapter_dir), is_trainable=True)
            logger.info("Loaded adapter from previous iteration: %s", input.adapter_path)
        else:
            # First round: attach a fresh LoRA adapter to the base model.
            target_modules = hp.get(
                "target_modules",
                ["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
            )
            model = engine.attach_adapter(
                model,
                r=hp.get("r", 16),
                lora_alpha=hp.get("lora_alpha", 16),
                lora_dropout=hp.get("lora_dropout", 0),
                target_modules=target_modules,
            )

        phase = f"iter_{iteration}"
        metrics = _train_sft(
            model,
            tokenizer,
            dataset,
            hp,
            job_id,
            max_seq_length,
            phase=phase,
            tenant_id=input.tenant_id,
            s3=s3,
            bucket=s3_bucket,
        )

        adapter_dir = tmpdir_path / "adapter"
        engine.save_adapter(model, tokenizer, adapter_dir)

        ckpt_s3_path = s3_paths.checkpoint_prefix(input.tenant_id, job_id) + f"iter-{iteration}/"
        adapter_size = _upload_adapter(adapter_dir, ckpt_s3_path, s3, s3_bucket)

        logger.info(
            "Iteration %d complete for job %s, checkpoint: %s", iteration, job_id, ckpt_s3_path
        )

        return TrainSftRoundOutput(
            adapter_path=ckpt_s3_path,
            adapter_size_bytes=adapter_size,
            metrics=metrics,
        )


async def run_evaluate_holdout_core(
    input: EvaluateHoldoutInput,
    *,
    s3,
    s3_bucket: str,
    settings,
) -> EvaluateHoldoutOutput:
    """Pure-compute holdout eval — needs only S3. No Postgres, no Redis-required.

    Loads the adapter from this iteration's checkpoint, evaluates on the
    validation split, returns eval_loss. Streams progress via the configured
    metrics sink (log by default remotely; Redis if a reachable APP_REDIS_URL
    is set). Runs identically in-process or inside a remote Modal container.
    """
    engine = get_engine(settings)
    hp = input.hyperparams
    job_id = input.training_job_id
    iteration = input.iteration

    _get_metrics_collector(settings)

    _stream_metric(
        job_id,
        {
            "event": "eval_begin",
            "phase": f"eval_iter_{iteration}",
            "timestamp": datetime.now(UTC).isoformat(),
        },
    )

    job_prefix = job_id[:8]
    with tempfile.TemporaryDirectory(prefix=f"eval-holdout-{job_prefix}-") as tmpdir:
        tmpdir_path = Path(tmpdir)

        # Download validation dataset
        val_s3_path = input.dataset_path.replace(".jsonl", "_val.jsonl")
        val_local = tmpdir_path / "val.jsonl"
        _download_dataset(val_s3_path, val_local, s3, s3_bucket)
        val_dataset = _load_chatml_dataset(val_local)
        logger.info("Loaded validation set: %d examples", len(val_dataset))

        _stream_metric(
            job_id,
            {
                "event": "eval_dataset_loaded",
                "phase": f"eval_iter_{iteration}",
                "val_examples": str(len(val_dataset)),
                "timestamp": datetime.now(UTC).isoformat(),
            },
        )

        # Load base model, then load THIS iteration's trained adapter from its
        # checkpoint. Use PeftModel.from_pretrained (same as run_evaluation_core)
        # so the saved adapter's own config + weights are restored exactly. The
        # earlier attach_adapter()+load_adapter("default") approach created a
        # fresh random adapter named "default" and then tried to load saved
        # weights into that same name, which fails with a state_dict mismatch.
        max_seq_length = hp.get("max_seq_length", 2048)
        load_in_4bit = input.method == "qlora"
        model, tokenizer = engine.load_model(
            model_name=input.base_model,
            max_seq_length=max_seq_length,
            load_in_4bit=load_in_4bit,
        )

        adapter_dir = tmpdir_path / "adapter"
        adapter_dir.mkdir(parents=True)
        _download_adapter(input.adapter_path, adapter_dir, s3, s3_bucket)

        from peft import PeftModel

        model = PeftModel.from_pretrained(model, str(adapter_dir))

        safe_heartbeat(f"eval_iter_{iteration}_running")

        # Evaluate
        eval_loss = _evaluate_on_holdout(model, tokenizer, val_dataset, hp, max_seq_length)

        _stream_metric(
            job_id,
            {
                "event": "eval_end",
                "phase": f"eval_iter_{iteration}",
                "eval_loss": str(round(eval_loss, 6)),
                "timestamp": datetime.now(UTC).isoformat(),
            },
        )

        logger.info(
            "Holdout eval iteration %d for job %s: eval_loss=%.4f", iteration, job_id, eval_loss
        )

        return EvaluateHoldoutOutput(
            eval_loss=eval_loss,
            metrics={"iteration": iteration, "eval_loss": eval_loss},
        )


async def _run_training(input: StartTrainingInput, infra: InfraContainer) -> StartTrainingOutput:
    """DB-coupled wrapper: resolve tenant llm_config, then delegate to the core.

    Retained for in-process callers (iterative rounds) that pass an infra container.
    """
    from src.tenant_config import get_tenant_llm_config

    llm_config = await get_tenant_llm_config(
        db=infra.db,
        tenant_id=input.tenant_id,
        default_api_base_url=infra.settings.llm_api_base_url,
        default_api_key=infra.settings.llm_api_key,
        default_model=infra.settings.llm_model,
        encryption_key=infra.settings.settings_encryption_key,
        settings=infra.settings,
    )
    return await run_training_core(
        input,
        s3=infra.s3,
        s3_bucket=infra.s3_bucket,
        settings=infra.settings,
        llm_config=llm_config,
    )


# -- Training Strategies --


@register_strategy("quick")
class QuickStrategy:
    """SFT-only training — fastest iteration."""

    name = "quick"

    def execute(self, model, tokenizer, dataset, hp, job_id, max_seq_length, **kwargs):
        tenant_id = kwargs.get("tenant_id")
        s3 = kwargs.get("s3")
        bucket = kwargs.get("bucket")
        return _train_sft(
            model,
            tokenizer,
            dataset,
            hp,
            job_id,
            max_seq_length,
            tenant_id=tenant_id,
            s3=s3,
            bucket=bucket,
        )


@register_strategy("distill")
class DistillStrategy(QuickStrategy):
    """SFT on teacher-written data.

    The training pass is identical to `quick` — what makes a run a
    distillation is upstream (the teacher generated the dataset) and
    downstream (the teacher-parity evaluation suite). Deliberately NOT
    judge-backed: training itself never calls the teacher or a judge.
    """

    name = "distill"


@register_strategy("distill_logit")
class DistillLogitStrategy:
    """Distillation from the teacher's stored per-token distributions.

    Never named in the public API: `TrainingMode` stays `distill` and
    `resolve_strategy_key` selects this strategy for `distill_method = logit`.
    """

    name = "distill_logit"

    def execute(self, model, tokenizer, dataset, hp, job_id, max_seq_length, **kwargs):
        # `dataset` goes unused on purpose. The artifacts carry the exact token ids
        # the teacher scored, and re-tokenizing the same text here could shift
        # every target against distributions that cannot be recomputed.
        return _train_distill_logit(
            model,
            tokenizer,
            hp,
            job_id,
            max_seq_length,
            tenant_id=kwargs.get("tenant_id"),
            base_model=kwargs.get("base_model"),
            s3=kwargs.get("s3"),
            bucket=kwargs.get("bucket"),
            settings=kwargs.get("settings"),
        )


@register_strategy("distill_on_policy")
class DistillOnPolicyStrategy:
    """Distillation from the teacher's grading of the student's own output.

    Never named in the public API: `TrainingMode` stays `distill` and
    `resolve_strategy_key` selects this strategy for `distill_method = on_policy`.

    Declares its engine because it cannot run under any other: the teacher is a
    vLLM process in this container, and Unsloth cannot be installed beside vLLM.

    `runs_resident_teacher` is what makes the caller partition the container's GPUs
    before loading the student, which has to happen outside any strategy — see
    `_reserve_student_devices`.
    """

    name = "distill_on_policy"
    required_engine = "transformers"
    runs_resident_teacher = True

    def execute(self, model, tokenizer, dataset, hp, job_id, max_seq_length, **kwargs):
        return _train_distill_on_policy(
            model,
            tokenizer,
            dataset,
            hp,
            job_id,
            max_seq_length,
            tenant_id=kwargs.get("tenant_id"),
            s3=kwargs.get("s3"),
            bucket=kwargs.get("bucket"),
            settings=kwargs.get("settings"),
            teacher_devices=kwargs.get("teacher_devices"),
        )


@register_strategy("aligned")
class AlignedStrategy:
    """SFT → DPO pipeline for production quality alignment."""

    name = "aligned"

    def execute(self, model, tokenizer, dataset, hp, job_id, max_seq_length, **kwargs):
        tenant_id = kwargs.get("tenant_id")
        s3 = kwargs.get("s3")
        bucket = kwargs.get("bucket")
        metrics_sft = _train_sft(
            model,
            tokenizer,
            dataset,
            hp,
            job_id,
            max_seq_length,
            phase="sft",
            tenant_id=tenant_id,
            s3=s3,
            bucket=bucket,
        )
        llm_config = kwargs.get("llm_config")
        metrics_dpo = _train_dpo(
            model,
            tokenizer,
            dataset,
            hp,
            job_id,
            max_seq_length,
            llm_config=llm_config,
            settings=kwargs.get("settings"),
        )
        return {**metrics_sft, "dpo": metrics_dpo}


@register_strategy("reasoning")
class ReasoningStrategy:
    """SFT → GRPO pipeline for reward-guided reasoning."""

    name = "reasoning"

    def execute(self, model, tokenizer, dataset, hp, job_id, max_seq_length, **kwargs):
        tenant_id = kwargs.get("tenant_id")
        s3 = kwargs.get("s3")
        bucket = kwargs.get("bucket")
        llm_config = kwargs.get("llm_config")
        metrics_sft = _train_sft(
            model,
            tokenizer,
            dataset,
            hp,
            job_id,
            max_seq_length,
            phase="sft",
            tenant_id=tenant_id,
            s3=s3,
            bucket=bucket,
        )
        metrics_grpo = _train_grpo(
            model,
            tokenizer,
            dataset,
            hp,
            job_id,
            max_seq_length,
            llm_config=llm_config,
            settings=kwargs.get("settings"),
        )
        return {**metrics_sft, "grpo": metrics_grpo}


# -- SFT Training --


def _train_sft(
    model,
    tokenizer,
    dataset,
    hp,
    job_id,
    max_seq_length,
    phase=None,
    tenant_id=None,
    s3=None,
    bucket=None,
):
    """Run SFT (Supervised Fine-Tuning) training."""
    from trl import SFTConfig, SFTTrainer

    # Render messages -> text with the model's own chat template just before
    # training, now that the tokenizer is available.
    dataset = _render_sft_dataset(dataset, tokenizer)

    phase_prefix = f"{phase}_" if phase else ""
    CallbackClass = _build_callback_class()
    callback = CallbackClass(job_id, phase=phase or "sft")

    batch_size = hp.get("per_device_train_batch_size", 2)
    grad_accum = hp.get("gradient_accumulation_steps", 4)
    epochs = hp.get("num_train_epochs", 3)
    warmup_steps = _resolve_warmup_steps(
        configured=hp.get("warmup_steps", 10),
        dataset_rows=len(dataset),
        batch_size=batch_size,
        grad_accum=grad_accum,
        epochs=epochs,
    )

    save_steps = hp.get("save_steps", 100)
    enable_checkpoints = tenant_id is not None
    callbacks = [callback]
    if enable_checkpoints:
        CkptClass = _build_checkpoint_callback_class(tenant_id, job_id, s3, bucket)
        callbacks.append(CkptClass())

    training_args = SFTConfig(
        output_dir=f"/tmp/sft-{job_id[:8]}",
        per_device_train_batch_size=batch_size,
        gradient_accumulation_steps=grad_accum,
        num_train_epochs=epochs,
        warmup_steps=warmup_steps,
        learning_rate=hp.get("learning_rate", 2e-4),
        optim=hp.get("optim", "adamw_8bit"),
        lr_scheduler_type=hp.get("lr_scheduler_type", "cosine"),
        max_seq_length=max_seq_length,
        logging_steps=1,
        save_strategy="steps" if enable_checkpoints else "no",
        save_steps=save_steps,
        save_total_limit=3,
        fp16=not _is_bf16_supported(),
        bf16=_is_bf16_supported(),
        seed=42,
        report_to="none",
    )

    trainer = SFTTrainer(
        model=model,
        processing_class=tokenizer,
        train_dataset=dataset,
        args=training_args,
        callbacks=callbacks,
    )

    train_result = trainer.train()

    return {
        f"{phase_prefix}train_loss": train_result.training_loss,
        f"{phase_prefix}train_steps": train_result.global_step,
        f"{phase_prefix}train_runtime": train_result.metrics.get("train_runtime", 0),
        f"{phase_prefix}train_samples_per_second": train_result.metrics.get(
            "train_samples_per_second", 0
        ),
    }


# -- Logit Distillation Training --


def _train_distill_logit(
    model,
    tokenizer,
    hp,
    job_id,
    max_seq_length,
    *,
    tenant_id=None,
    base_model=None,
    s3=None,
    bucket=None,
    settings=None,
):
    """Train against the teacher's stored top-k distributions instead of text.

    Shards are streamed one at a time: a dataset's artifacts are far larger than
    the dataset itself, and holding them all would cap the dataset size a GPU can
    train on for no reason.
    """
    from transformers import TrainingArguments

    prefix = hp.get(_TEACHER_ARTIFACTS_HYPERPARAM)
    if not prefix:
        raise ApplicationError(
            "High-fidelity distillation needs the teacher's scored artifacts, "
            "and this job was started without them",
            non_retryable=True,
        )
    if base_model is None:
        raise ApplicationError(
            "Cannot verify teacher artifacts without the student model id",
            non_retryable=True,
        )

    loss_config = DistillLossConfig.from_hyperparams(hp)
    batch_size = hp.get("per_device_train_batch_size", 2)
    grad_accum = hp.get("gradient_accumulation_steps", 4)
    epochs = hp.get("num_train_epochs", 3)

    with tempfile.TemporaryDirectory(prefix=f"distill-logit-{job_id[:8]}-") as tmpdir:
        tmpdir_path = Path(tmpdir)
        manifest = _load_teacher_manifest(prefix, tmpdir_path, s3, bucket)
        _verify_teacher_manifest(
            manifest, base_model=base_model, tokenizer=tokenizer, settings=settings
        )

        records = int(manifest["totals"]["records"])
        if records < 1:
            raise ApplicationError(
                "The teacher scored no records for this dataset", non_retryable=True
            )
        effective_batch = max(1, batch_size * grad_accum)
        max_steps = max(1, int(math.ceil(records / effective_batch) * epochs))

        enable_checkpoints = tenant_id is not None
        callbacks = [_build_callback_class()(job_id, phase="distill_logit")]
        if enable_checkpoints:
            callbacks.append(_build_checkpoint_callback_class(tenant_id, job_id, s3, bucket)())

        training_args = TrainingArguments(
            output_dir=f"/tmp/distill-logit-{job_id[:8]}",
            per_device_train_batch_size=batch_size,
            gradient_accumulation_steps=grad_accum,
            max_steps=max_steps,
            warmup_steps=_resolve_warmup_steps(
                configured=hp.get("warmup_steps", 10),
                dataset_rows=records,
                batch_size=batch_size,
                grad_accum=grad_accum,
                epochs=epochs,
            ),
            learning_rate=hp.get("learning_rate", 2e-4),
            optim=hp.get("optim", "adamw_8bit"),
            lr_scheduler_type=hp.get("lr_scheduler_type", "cosine"),
            logging_steps=1,
            save_strategy="steps" if enable_checkpoints else "no",
            save_steps=hp.get("save_steps", 100),
            save_total_limit=3,
            fp16=not _is_bf16_supported(),
            bf16=_is_bf16_supported(),
            seed=42,
            report_to="none",
            remove_unused_columns=False,
            label_names=["labels"],
        )

        dataset = _build_artifact_dataset_class()(
            manifest=manifest,
            prefix=prefix,
            tmpdir=tmpdir_path,
            s3=s3,
            bucket=bucket,
            passes=max(1, math.ceil(epochs)),
            max_seq_length=max_seq_length,
        )
        pad_token_id = _padding_token_id(tokenizer)
        trainer = _build_distill_trainer_class(loss_config)(
            model=model,
            args=training_args,
            train_dataset=dataset,
            data_collator=lambda batch: collate_distill_batch(batch, pad_token_id=pad_token_id),
            callbacks=callbacks,
        )
        train_result = trainer.train()

    logger.info(
        "Logit distillation finished for job %s: %d steps over %d scored records",
        job_id,
        train_result.global_step,
        records,
    )
    # The prefix is recorded so evaluation can measure the student against the
    # same distributions it trained on; nothing else in the run needs it back.
    return {
        _TEACHER_ARTIFACTS_HYPERPARAM: prefix,
        "distill_logit_train_loss": train_result.training_loss,
        "distill_logit_train_steps": train_result.global_step,
        "distill_logit_train_runtime": train_result.metrics.get("train_runtime", 0),
        "distill_logit_scored_positions": int(manifest["totals"]["scored_positions"]),
        "kd_alpha": loss_config.kd_alpha,
        "ce_alpha": loss_config.ce_alpha,
        "kd_temperature": loss_config.temperature,
        "tail_beta": loss_config.tail_beta,
    }


def _build_teacher_liveness_callback_class(server):
    """A callback that stops training the moment the teacher dies.

    Built lazily so importing this module does not require transformers.

    Without it, a dead teacher becomes a run that keeps taking gradient steps
    against failed requests and finishes reporting success — the worst outcome
    available, because the model looks trained and learned nothing.
    """
    from transformers import TrainerCallback

    class TeacherLivenessCallback(TrainerCallback):
        def on_step_end(self, args, state, control, **kwargs):
            server.check_alive()
            return control

    return TeacherLivenessCallback


def _reserve_student_devices(strategy):
    """Confine this process to the student's card, before any CUDA state exists.

    `CUDA_VISIBLE_DEVICES` is consulted once, when a process first touches CUDA,
    and is inert afterwards. So this cannot be done from inside the strategy: by
    then the student's weights are loaded, and they went to device 0 — the card the
    teacher is about to fill to 90% of its memory.

    Reads the real device set rather than trusting the requested GPU class, because
    a container that came up with fewer cards than were asked for must fail saying
    so instead of putting both models on one.

    Returns the teacher's device ids, or None for a strategy with no resident
    teacher — which is every other strategy, and they keep the whole container.
    """
    if not getattr(strategy, "runs_resident_teacher", False):
        return None

    from src.teacher.server import TeacherServerError, container_gpu_ids, split_devices

    try:
        teacher_devices, student_devices = split_devices(container_gpu_ids())
    except TeacherServerError as exc:
        raise ApplicationError(str(exc), non_retryable=True) from exc

    os.environ["CUDA_VISIBLE_DEVICES"] = ",".join(str(device) for device in student_devices)

    import torch

    visible = torch.cuda.device_count()
    if visible != len(student_devices):
        # Reaching CUDA before this point fixes the device set permanently, and no
        # later assignment can move the student off the teacher's card. Refusing is
        # the only outcome left that does not end in an out-of-memory kill.
        raise ApplicationError(
            f"Could not confine training to {len(student_devices)} GPU(s): this "
            f"process already sees {visible}. On-policy distillation needs a worker "
            f"that has not run other GPU work first.",
            non_retryable=True,
        )

    logger.info(
        "Reserved GPU(s) %s for the student, %s for the teacher",
        student_devices,
        teacher_devices,
    )
    return teacher_devices


def _train_distill_on_policy(
    model,
    tokenizer,
    dataset,
    hp,
    job_id,
    max_seq_length,
    *,
    tenant_id=None,
    s3=None,
    bucket=None,
    settings=None,
    teacher_devices=None,
):
    """Train the student on the teacher's grading of the student's own output.

    The teacher runs as a subprocess on its own GPU in this container, and the
    trainer talks to it over loopback. See `src/teacher/server.py` for why that is
    a sidecar rather than a separately scheduled server.

    `teacher_devices` is assigned by the caller, not here: the student's own card
    has to be claimed before its weights are loaded, which is already past by the
    time this runs.
    """
    from src.activities.on_policy import OnPolicyConfigError, plan_on_policy
    from src.teacher.server import TeacherServerConfig, TeacherServerError, teacher_server

    teacher_model = hp.get("teacher_model")
    if not teacher_model:
        raise ApplicationError(
            "On-policy distillation needs a teacher to grade against, and this job "
            "was started without one",
            non_retryable=True,
        )

    precision = hp.get("teacher_precision") or "bf16"
    if precision not in _TEACHER_DTYPE_BY_PRECISION:
        raise ApplicationError(
            f"A served teacher cannot run at {precision}: quantized weights need a "
            f"separate vLLM quantization path that this run does not set up. "
            f"Supported: {', '.join(_TEACHER_DTYPE_BY_PRECISION)}.",
            non_retryable=True,
        )

    if not teacher_devices:
        raise ApplicationError(
            "On-policy distillation reached training without a GPU reserved for the "
            "teacher, so the teacher would have shared the student's card.",
            non_retryable=True,
        )

    teacher_config = TeacherServerConfig(
        model=teacher_model,
        revision=hp.get("teacher_revision") or None,
        devices=tuple(teacher_devices),
        dtype=_TEACHER_DTYPE_BY_PRECISION[precision],
        max_model_len=hp.get("teacher_max_model_len") or max_seq_length,
        startup_timeout_secs=int(
            hp.get("teacher_startup_timeout_secs", _DEFAULT_TEACHER_STARTUP_TIMEOUT_SECS)
        ),
    )

    with tempfile.TemporaryDirectory(prefix=f"distill-onpolicy-{job_id[:8]}-") as tmpdir:
        try:
            with teacher_server(teacher_config) as server:
                plan = plan_on_policy(
                    hp,
                    teacher_model=teacher_model,
                    teacher_url=server.base_url,
                    teacher_revision=teacher_config.revision,
                )
                metrics = _run_on_policy_trainer(
                    model=model,
                    tokenizer=tokenizer,
                    dataset=dataset,
                    plan=plan,
                    hp=hp,
                    job_id=job_id,
                    output_dir=tmpdir,
                    server=server,
                    tenant_id=tenant_id,
                    s3=s3,
                    bucket=bucket,
                )
        except OnPolicyConfigError as exc:
            raise ApplicationError(str(exc), non_retryable=True) from exc
        except TeacherServerError as exc:
            # Non-retryable: a retry re-books the same container and the same
            # teacher, so it fails the same way while charging for it again.
            raise ApplicationError(str(exc), non_retryable=True) from exc

    logger.info(
        "On-policy distillation finished for job %s: %s steps against teacher %s",
        job_id,
        metrics.get("on_policy_train_steps"),
        teacher_model,
    )
    return metrics


def _run_on_policy_trainer(
    *,
    model,
    tokenizer,
    dataset,
    plan,
    hp,
    job_id,
    output_dir,
    server,
    tenant_id,
    s3,
    bucket,
):
    """Drive TRL's on-policy trainer for one run.

    Separated from teacher startup so the import of `trl.experimental` — which
    only exists in the on-policy image — happens after the teacher is confirmed
    healthy, and so this half can be read without the lifecycle noise.
    """
    from trl.experimental.iw_opd import IWOPDConfig, IWOPDTrainer

    from src.activities.on_policy import trainer_config_kwargs

    callbacks = [
        _build_callback_class()(job_id, phase="distill_on_policy"),
        _build_teacher_liveness_callback_class(server)(),
    ]
    if tenant_id is not None:
        callbacks.append(_build_checkpoint_callback_class(tenant_id, job_id, s3, bucket)())

    config = IWOPDConfig(**trainer_config_kwargs(plan, output_dir=output_dir, hp=hp))
    trainer = IWOPDTrainer(
        model=model,
        args=config,
        train_dataset=dataset,
        processing_class=tokenizer,
        callbacks=callbacks,
    )
    train_result = trainer.train()

    return {
        "on_policy_train_loss": train_result.training_loss,
        "on_policy_train_steps": train_result.global_step,
        "on_policy_train_runtime": train_result.metrics.get("train_runtime", 0),
        "on_policy_teacher_model": plan.teacher_model,
        "on_policy_objective": plan.objective,
        "on_policy_beta": plan.beta,
        "on_policy_lambda": plan.lmbda,
        "on_policy_rollout_temperature": plan.temperature,
        "on_policy_loss_top_k": plan.loss_top_k,
    }


def _load_teacher_manifest(prefix: str, tmpdir: Path, s3, bucket: str) -> dict:
    local = tmpdir / "manifest.json"
    _download_dataset(prefix + "manifest.json", local, s3, bucket)
    return json.loads(local.read_text())


def _verify_teacher_manifest(manifest: dict, *, base_model: str, tokenizer, settings=None) -> None:
    """Refuse artifacts that were not computed for this student's tokenization.

    The extraction job already checked this before spending a GPU; checking again
    here is what makes a catalog edit or a swapped student between the two passes
    a failed run rather than a model trained on shifted targets.
    """
    from src.teacher.artifacts import manifest_matches
    from src.teacher.rendering import rendering_fingerprint
    from src.teacher.tokenizer_identity import compute_tokenizer_hashes

    hf_token = getattr(settings, "hf_token", "") or ""
    hashes = compute_tokenizer_hashes(base_model, hf_token=hf_token)
    if not manifest_matches(
        manifest,
        tokenizer_hash=hashes.combined_hash,
        rendering_fingerprint=rendering_fingerprint(tokenizer),
    ):
        logger.error(
            "Teacher artifacts reject student %s: manifest tokenizer_hash=%s fingerprint=%s",
            base_model,
            manifest.get("tokenizer_hash"),
            manifest.get("rendering_fingerprint"),
        )
        raise ApplicationError(_TEACHER_ARTIFACT_MISMATCH, non_retryable=True)


def _padding_token_id(tokenizer) -> int:
    """Token used to pad short records in a batch — never read by the loss.

    Padded positions are attention-masked and label-masked, so any real id does;
    the eos fallback exists only because some base tokenizers ship no pad token.
    """
    for candidate in (tokenizer.pad_token_id, tokenizer.eos_token_id):
        if candidate is not None:
            return int(candidate)
    raise ApplicationError(
        "Tokenizer has neither a pad nor an eos token, so batches cannot be padded",
        non_retryable=True,
    )


def _truncate_record_view(view: dict, max_seq_length: int):
    """Fit one scored record into the model's context, or drop it.

    Cutting from the right is safe: every position that survives was scored by the
    teacher conditioned only on tokens before it, so the remaining targets are
    still exactly aligned. A record whose prompt alone fills the context has
    nothing left to supervise and is dropped.
    """
    length = len(view["input_ids"])
    if length <= max_seq_length:
        return view

    completion_start = int(view["completion_start"])
    if completion_start >= max_seq_length:
        return None

    kept = max_seq_length - completion_start
    return {
        "input_ids": view["input_ids"][:max_seq_length],
        "completion_start": completion_start,
        "token_ids": view["token_ids"][:kept],
        "logprobs": view["logprobs"][:kept],
        "support_len": view["support_len"][:kept],
        "tail_mass": view["tail_mass"][:kept],
    }


def _build_artifact_dataset_class():
    """Build the streaming artifact dataset as a torch IterableDataset subclass."""
    from torch.utils.data import IterableDataset

    class TeacherArtifactDataset(IterableDataset):
        """Yields one scored record at a time, holding at most one shard on disk."""

        def __init__(self, *, manifest, prefix, tmpdir, s3, bucket, passes, max_seq_length):
            super().__init__()
            self.shards = manifest["shards"]
            self.prefix = prefix
            self.tmpdir = tmpdir
            self.s3 = s3
            self.bucket = bucket
            self.passes = passes
            self.max_seq_length = max_seq_length

        def __iter__(self):
            for _ in range(self.passes):
                for shard in self.shards:
                    yield from self._iter_shard(shard)

        def _iter_shard(self, shard):
            from src.teacher.artifacts import read_shard, record_view

            local = self.tmpdir / shard["name"]
            _download_dataset(self.prefix + shard["name"], local, self.s3, self.bucket)
            dropped = 0
            try:
                arrays = read_shard(str(local))
                for position in range(int(shard["records"])):
                    view = _truncate_record_view(record_view(arrays, position), self.max_seq_length)
                    if view is None:
                        dropped += 1
                        continue
                    yield view
            finally:
                local.unlink(missing_ok=True)
            if dropped:
                logger.warning(
                    "Shard %s: %d records had no room to supervise within %d tokens",
                    shard["name"],
                    dropped,
                    self.max_seq_length,
                )

    return TeacherArtifactDataset


def _build_distill_trainer_class(loss_config: DistillLossConfig):
    """Build a Trainer whose loss is KL against the teacher's stored distributions."""
    from transformers import Trainer

    class DistillLogitTrainer(Trainer):
        def compute_loss(self, model, inputs, return_outputs=False, **kwargs):
            labels = inputs.pop("labels")
            teacher = {
                "teacher_token_ids": inputs.pop("teacher_token_ids"),
                "teacher_logprobs": inputs.pop("teacher_logprobs"),
                "teacher_support_len": inputs.pop("teacher_support_len"),
                "teacher_tail_mass": inputs.pop("teacher_tail_mass"),
            }
            outputs = model(**inputs)
            parts = distillation_loss(
                student_logits=outputs.logits,
                labels=labels,
                config=loss_config,
                **teacher,
            )
            return (parts.loss, outputs) if return_outputs else parts.loss

    return DistillLogitTrainer


# -- DPO Training --


def _train_dpo(model, tokenizer, dataset, hp, job_id, max_seq_length, llm_config, settings=None):
    """Run DPO (Direct Preference Optimization) training."""
    from trl import DPOConfig, DPOTrainer

    CallbackClass = _build_callback_class()
    callback = CallbackClass(job_id, phase="dpo")

    dpo_dataset = _create_dpo_pairs(model, dataset, tokenizer, hp, llm_config, settings)

    dpo_epochs = max(1, hp.get("num_train_epochs", 3) // 2)
    training_args = DPOConfig(
        output_dir=f"/tmp/dpo-{job_id[:8]}",
        per_device_train_batch_size=hp.get("per_device_train_batch_size", 2),
        gradient_accumulation_steps=hp.get("gradient_accumulation_steps", 4),
        num_train_epochs=dpo_epochs,
        learning_rate=hp.get("learning_rate", 2e-4) / 10,
        optim=hp.get("optim", "adamw_8bit"),
        lr_scheduler_type="cosine",
        max_length=max_seq_length,
        logging_steps=1,
        save_strategy="no",
        fp16=not _is_bf16_supported(),
        bf16=_is_bf16_supported(),
        report_to="none",
    )

    trainer = DPOTrainer(
        model=model,
        processing_class=tokenizer,
        train_dataset=dpo_dataset,
        args=training_args,
        callbacks=[callback],
    )

    train_result = trainer.train()

    return {
        "dpo_loss": train_result.training_loss,
        "dpo_steps": train_result.global_step,
        "dpo_runtime": train_result.metrics.get("train_runtime", 0),
    }


# -- GRPO Training --


def _build_grpo_reward(judge):
    """Build the GRPO reward function with per-record dispatch.

    A record whose gold turn is a tool call is scored deterministically
    against its schema and reference; everything else keeps the LLM-judge
    reasoning reward. Both live on [-1, 1], and GRPO compares rewards only
    within a prompt's generation group, so mixed datasets are fine.
    """
    from src.activities.tool_call_reward import score_tool_call_completion

    def reasoning_reward(
        completions: list[str],
        tools_json: list[str] | None = None,
        ref_calls_json: list[str] | None = None,
        **kwargs,
    ) -> list[float]:
        rewards = []
        for i, completion in enumerate(completions):
            ref_raw = ref_calls_json[i] if ref_calls_json else ""
            if ref_raw:
                tools_raw = tools_json[i] if tools_json else ""
                rewards.append(
                    score_tool_call_completion(
                        completion,
                        tools=json.loads(tools_raw) if tools_raw else [],
                        reference_calls=json.loads(ref_raw),
                    )
                )
            else:
                rewards.append(judge.score_reasoning(completion))
        return rewards

    return reasoning_reward


def _train_grpo(model, tokenizer, dataset, hp, job_id, max_seq_length, llm_config, settings=None):
    """Run GRPO (Group Relative Policy Optimization) for reasoning tasks."""
    from trl import GRPOConfig, GRPOTrainer

    CallbackClass = _build_callback_class()
    callback = CallbackClass(job_id, phase="grpo")

    grpo_dataset = _create_grpo_prompts(dataset, tokenizer)

    grpo_epochs = max(1, hp.get("num_train_epochs", 3) // 2)
    training_args = GRPOConfig(
        output_dir=f"/tmp/grpo-{job_id[:8]}",
        per_device_train_batch_size=hp.get("per_device_train_batch_size", 2),
        gradient_accumulation_steps=hp.get("gradient_accumulation_steps", 4),
        num_train_epochs=grpo_epochs,
        learning_rate=hp.get("learning_rate", 2e-4) / 10,
        optim=hp.get("optim", "adamw_8bit"),
        lr_scheduler_type="cosine",
        max_completion_length=max_seq_length // 2,
        logging_steps=1,
        save_strategy="no",
        fp16=not _is_bf16_supported(),
        bf16=_is_bf16_supported(),
        report_to="none",
    )

    judge = OpenAICompatibleJudge(
        api_base=llm_config.api_base_url,
        api_key=llm_config.api_key,
        model=llm_config.model,
        max_retries=getattr(settings, "judge_max_retries", 3),
        on_failure=getattr(settings, "judge_on_failure", "error"),
    )

    reasoning_reward = _build_grpo_reward(judge)

    trainer = GRPOTrainer(
        model=model,
        processing_class=tokenizer,
        train_dataset=grpo_dataset,
        reward_funcs=[reasoning_reward],
        args=training_args,
        callbacks=[callback],
    )

    train_result = trainer.train()

    return {
        "grpo_loss": train_result.training_loss,
        "grpo_steps": train_result.global_step,
        "grpo_runtime": train_result.metrics.get("train_runtime", 0),
    }


def _evaluate_on_holdout(model, tokenizer, val_dataset, hp, max_seq_length) -> float:
    """Run evaluation on a hold-out validation set and return eval_loss."""
    from trl import SFTConfig, SFTTrainer

    # Render with the model's own chat template so holdout loss is measured on
    # the same formatting used for training and serving.
    val_dataset = _render_sft_dataset(val_dataset, tokenizer)

    eval_args = SFTConfig(
        output_dir="/tmp/eval-holdout",
        per_device_eval_batch_size=hp.get("per_device_train_batch_size", 2),
        max_seq_length=max_seq_length,
        fp16=not _is_bf16_supported(),
        bf16=_is_bf16_supported(),
        report_to="none",
    )

    # train_dataset is required even for an eval-only run: Unsloth's trainer
    # init calls fix_zero_training_loss(), which does len(train_dataset) and
    # raises TypeError on None. We only ever call .evaluate() (never .train()),
    # so reusing val_dataset as the train_dataset is inert — no training happens.
    trainer = SFTTrainer(
        model=model,
        processing_class=tokenizer,
        train_dataset=val_dataset,
        eval_dataset=val_dataset,
        args=eval_args,
    )

    eval_result = trainer.evaluate()
    return eval_result.get("eval_loss", 0.0)


# -- Helpers --

_metrics_collector = None


def _get_metrics_collector(settings=None):
    """Get or create the module-level MetricsCollector (backend selected from settings)."""
    global _metrics_collector  # noqa: PLW0603
    if _metrics_collector is None:
        from src.backends.metrics_collector import get as get_collector

        if settings is None:
            raise RuntimeError("Settings required for first MetricsCollector initialization")
        _metrics_collector = get_collector(settings.metrics_backend, settings.redis_url)
    return _metrics_collector


def _get_gpu_metrics() -> dict:
    """Collect current GPU metrics using pynvml. Returns empty dict on failure."""
    try:
        import pynvml

        pynvml.nvmlInit()
        handle = pynvml.nvmlDeviceGetHandleByIndex(0)

        util = pynvml.nvmlDeviceGetUtilizationRates(handle)
        mem = pynvml.nvmlDeviceGetMemoryInfo(handle)
        temp = pynvml.nvmlDeviceGetTemperature(handle, pynvml.NVML_TEMPERATURE_GPU)

        return {
            "gpu_utilization": str(util.gpu),
            "gpu_memory_used_mb": str(mem.used // (1024 * 1024)),
            "gpu_memory_total_mb": str(mem.total // (1024 * 1024)),
            "gpu_memory_pct": str(round(mem.used / mem.total * 100, 1)),
            "gpu_temperature_c": str(temp),
        }
    except Exception:
        return {}


def _resolve_warmup_steps(
    *,
    configured: int,
    dataset_rows: int,
    batch_size: int,
    grad_accum: int,
    epochs: float,
) -> int:
    """Cap warmup so it never swallows a short run.

    A fixed warmup (e.g. 10) exceeds the total step count on small datasets, so
    the learning rate never leaves its ramp and the adapter learns almost
    nothing. Keep warmup at a fraction of the actual schedule.
    """
    effective_batch = max(1, batch_size * grad_accum)
    steps_per_epoch = max(1, math.ceil(dataset_rows / effective_batch))
    total_steps = max(1, int(steps_per_epoch * epochs))
    capped = max(1, int(total_steps * _MAX_WARMUP_FRACTION))
    resolved = min(configured, capped)
    if resolved != configured:
        logger.info(
            "Capped warmup %d -> %d (total_steps=%d, rows=%d)",
            configured,
            resolved,
            total_steps,
            dataset_rows,
        )
    return resolved


def _build_callback_class():
    """Build MetricsStreamingCallback as a proper TrainerCallback subclass."""
    try:
        from transformers import TrainerCallback

        _base = TrainerCallback
    except ImportError:
        _base = object

    class MetricsStreamingCallback(_base):
        """HuggingFace TrainerCallback that streams metrics to Redis."""

        def __init__(self, job_id: str, phase: str = ""):
            if _base is not object:
                super().__init__()
            self.job_id = job_id
            self.phase = phase
            self._start_time: float | None = None
            self._start_step: int = 0

        def on_log(self, args, state, control, logs=None, **kwargs):
            if logs is None:
                return

            now = time.monotonic()
            current_step = state.global_step
            max_steps = state.max_steps or 0

            # Compute ETA from steps/second since training began
            eta_seconds: int | None = None
            steps_per_second: float = 0.0
            if self._start_time is not None and current_step > self._start_step:
                elapsed = now - self._start_time
                done_steps = current_step - self._start_step
                steps_per_second = done_steps / max(elapsed, 0.001)
                remaining = max(0, max_steps - current_step)
                if steps_per_second > 0 and remaining > 0:
                    eta_seconds = int(remaining / steps_per_second)

            metrics = {
                "step": str(current_step),
                "total_steps": str(max_steps),
                "epoch": str(round(state.epoch or 0, 2)),
                "loss": str(logs.get("loss", 0)),
                "learning_rate": str(logs.get("learning_rate", 0)),
                "grad_norm": str(logs.get("grad_norm", 0)),
                "phase": self.phase,
                "steps_per_second": str(round(steps_per_second, 3)),
                "timestamp": datetime.now(UTC).isoformat(),
            }
            if eta_seconds is not None:
                metrics["eta_seconds"] = str(eta_seconds)

            gpu = _get_gpu_metrics()
            if gpu:
                metrics.update(gpu)

            try:
                stream_key = f"training:metrics:{self.job_id}"
                _get_metrics_collector().record(stream_key, metrics, maxlen=10000)
            except Exception as e:
                logger.warning("Failed to stream metrics: %s", e)

            safe_heartbeat(f"step={current_step}/{max_steps}")

        def on_train_begin(self, args, state, control, **kwargs):
            self._start_time = time.monotonic()
            self._start_step = state.global_step
            _stream_metric(
                self.job_id,
                {
                    "event": "train_begin",
                    "phase": self.phase,
                    "total_steps": str(state.max_steps or 0),
                    "timestamp": datetime.now(UTC).isoformat(),
                },
            )

        def on_train_end(self, args, state, control, **kwargs):
            _stream_metric(
                self.job_id,
                {
                    "event": "train_end",
                    "phase": self.phase,
                    "total_steps": str(state.global_step),
                    "timestamp": datetime.now(UTC).isoformat(),
                },
            )

    return MetricsStreamingCallback


def _build_checkpoint_callback_class(tenant_id: str, job_id: str, s3, bucket: str):
    """Build a TrainerCallback that uploads checkpoints to S3 on save."""
    try:
        from transformers import TrainerCallback

        _base = TrainerCallback
    except ImportError:
        _base = object

    class CheckpointUploadCallback(_base):
        def __init__(self):
            if _base is not object:
                super().__init__()
            self.tenant_id = tenant_id
            self.job_id = job_id

        def on_save(self, args, state, control, **kwargs):
            try:
                ckpt_dir = Path(args.output_dir) / f"checkpoint-{state.global_step}"
                if ckpt_dir.is_dir():
                    s3_prefix = (
                        f"checkpoints/{self.tenant_id}/{self.job_id}/step-{state.global_step}/"
                    )
                    _upload_adapter(ckpt_dir, s3_prefix, s3, bucket)
                    logger.info("Uploaded checkpoint step %d to %s", state.global_step, s3_prefix)
            except Exception as e:
                logger.warning("Failed to upload checkpoint: %s", e)

    return CheckpointUploadCallback


def _stream_metric(job_id: str, data: dict):
    """Push a single metric event via the configured MetricsCollector."""
    try:
        stream_key = f"training:metrics:{job_id}"
        str_data = {k: str(v) for k, v in data.items()}
        _get_metrics_collector().record(stream_key, str_data, maxlen=10000)
    except Exception as e:
        logger.warning("Failed to stream metric event: %s", e)


def _download_dataset(s3_path: str, local_path: Path, s3, bucket: str):
    """Download dataset from S3 to local file."""
    s3.download_file(bucket, s3_path, str(local_path))
    logger.info("Downloaded dataset: %s -> %s", s3_path, local_path)


def _load_chatml_dataset(path: Path):
    """Load a chat JSONL dataset as raw message lists into a HuggingFace Dataset.

    Formatting is deliberately deferred until a tokenizer is available: each
    consumer renders the messages with the model's own chat template (via
    `chat_template.render_chat`) so training matches what the model is served,
    instead of a hardcoded ChatML string that only matched Qwen.
    """
    from datasets import Dataset

    rows = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                record = json.loads(line)
                # Keep each record's tool schema (empty when absent) so tool-call
                # trajectories render with their tool definitions downstream.
                rows.append(
                    {
                        "messages": record.get("messages", []),
                        "tools": record.get("tools", []),
                    }
                )

    return Dataset.from_list(rows)


def _dataset_column(dataset, name):
    """A dataset column's values, or None when the column does not exist."""
    try:
        return dataset[name]
    except KeyError:
        return None


def _render_sft_dataset(dataset, tokenizer):
    """Render a messages-form dataset into `{"text": ...}` for SFT / eval.

    Uses the model's own chat template so the full supervised example (prompt +
    assistant turn + the template's EOS) matches serve-time formatting. The
    record's `tools` schema is forwarded so tool-call trajectories train with
    their tool definitions rendered exactly as at serve time.
    """
    from src.activities.chat_template import render_chat

    def _map(row):
        return {
            "text": render_chat(
                tokenizer, row["messages"], add_generation_prompt=False, tools=row.get("tools")
            )
        }

    return dataset.map(_map, remove_columns=[c for c in dataset.column_names if c != "text"])


def _normalize_text(s: str) -> str:
    """Whitespace/case-insensitive normalization for near-identical detection."""
    return " ".join(s.lower().split())


def _create_dpo_pairs(model, dataset, tokenizer, hp, llm_config, settings=None):
    """Build on-policy, judge-filtered DPO preference pairs.

    For each SFT example the dataset's gold assistant response is the `chosen`
    answer; a fresh response sampled from the CURRENT model for the same prompt
    is the `rejected` answer. This is the standard on-policy construction (the
    model learns to prefer the gold answer over what it would produce itself),
    replacing the previous approach that used the gold response truncated to a
    third of its length — which only taught the model that longer is better.

    A judge filters the pairs: a pair is dropped when the sampled response is
    empty, near-identical to gold (no real preference signal), or judged
    *better* than gold (so we never label a genuinely-better answer as
    rejected). If the judge is unavailable, `on_failure='error'` (default) makes
    this raise rather than silently building noisy pairs.

    Configurable via hp: `max_dpo_pairs` (cap the number of pairs / generations,
    default all), `dpo_gen_max_new_tokens` (default 256), `dpo_judge_filter`
    (default True).
    """
    from datasets import Dataset

    from src.activities.chat_template import render_chat, split_prompt_and_response
    from src.backends.model_inference import get as get_inference

    judge = OpenAICompatibleJudge(
        api_base=llm_config.api_base_url,
        api_key=llm_config.api_key,
        model=llm_config.model,
        max_retries=getattr(settings, "judge_max_retries", 3),
        on_failure=getattr(settings, "judge_on_failure", "error"),
    )
    inference = get_inference("hf")

    max_pairs = hp.get("max_dpo_pairs")
    max_new_tokens = hp.get("dpo_gen_max_new_tokens", 256)
    judge_filter = hp.get("dpo_judge_filter", True)

    chosen_texts: list[str] = []
    rejected_texts: list[str] = []
    dropped = 0
    skipped_no_gold = 0

    messages_list = dataset["messages"]
    tools_list = _dataset_column(dataset, "tools") or [None] * len(messages_list)
    for messages, tools in zip(messages_list, tools_list):
        if max_pairs is not None and len(chosen_texts) >= max_pairs:
            break

        # Recover the prompt turns and gold answer from the structured messages
        # (never by string-splitting a formatted string — that only worked for
        # ChatML and broke on every other model's template).
        prompt_messages, gold = split_prompt_and_response(messages)
        if prompt_messages is None:
            skipped_no_gold += 1
            continue

        # Format the prompt with the model's own template so the sampled
        # ("rejected") response is generated exactly as the model is served.
        gen_prompt = render_chat(
            tokenizer, prompt_messages, add_generation_prompt=True, tools=tools
        )
        sampled = inference.generate(model, tokenizer, gen_prompt, max_new_tokens=max_new_tokens)
        sampled = sampled.strip()

        # Degenerate: no signal if empty or effectively equal to the gold answer.
        if not sampled or _normalize_text(sampled) == _normalize_text(gold):
            dropped += 1
            continue

        # Judge filter: keep only when gold is at least as good as the sample
        # (winner "A"=gold or "tie"); drop when the sample is judged better ("B").
        if judge_filter:
            winner = judge.compare_ab(gen_prompt, gold, sampled)
            if winner == "B":
                dropped += 1
                continue

        # chosen/rejected are the full templated conversations (prompt + turn),
        # sharing the gen_prompt prefix so the only difference is the answer.
        chosen_texts.append(
            render_chat(
                tokenizer,
                [*prompt_messages, {"role": "assistant", "content": gold}],
                tools=tools,
            )
        )
        rejected_texts.append(
            render_chat(
                tokenizer,
                [*prompt_messages, {"role": "assistant", "content": sampled}],
                tools=tools,
            )
        )

    if skipped_no_gold:
        logger.warning(
            "DPO: skipped %d record(s) whose final assistant turn has no text content "
            "(e.g. pure tool-call finals) — DPO needs a gold text response",
            skipped_no_gold,
        )
    logger.info(
        "DPO on-policy pairs: kept=%d dropped=%d (of %d examples)",
        len(chosen_texts),
        dropped,
        len(messages_list),
    )

    if not chosen_texts:
        raise ValueError(
            "DPO pair construction produced no usable preference pairs "
            "(all examples were degenerate or judge-filtered). Check the dataset "
            "and judge configuration."
        )

    return Dataset.from_dict({"chosen": chosen_texts, "rejected": rejected_texts})


def _create_grpo_prompts(dataset, tokenizer):
    """Build GRPO generation prompts from the dataset's messages.

    Each prompt is the conversation up to the assistant turn, formatted with the
    model's own chat template (including any system turn and the generation
    marker) — so the policy generates exactly as it will when served. The
    previous version fed the bare, untemplated user text and dropped the system
    prompt, so GRPO trained on inputs the deployed model never receives.
    """
    from datasets import Dataset

    from src.activities.chat_template import render_chat, split_prompt_and_response

    prompts = []
    skipped_no_gold = 0
    tool_records = 0
    messages_list = dataset["messages"]
    tools_list = _dataset_column(dataset, "tools") or [None] * len(messages_list)
    for messages, tools in zip(messages_list, tools_list):
        ref_calls = []
        prompt_messages, _gold = split_prompt_and_response(messages)
        if prompt_messages is None:
            last = messages[-1] if messages else {}
            if last.get("role") == "assistant" and last.get("tool_calls"):
                # A pure tool-call final has no gold text, but it can be scored
                # verifiably against the reference call — train on it instead
                # of skipping.
                prompt_messages = messages[:-1]
                ref_calls = last["tool_calls"]
                tool_records += 1
            elif any(m.get("role") == "assistant" for m in messages):
                # A final assistant turn with neither text nor tool calls has
                # nothing to learn from. Only truly prompt-only records (no
                # assistant turn at all) fall back to the whole conversation.
                skipped_no_gold += 1
                continue
            else:
                prompt_messages = messages
        if not prompt_messages:
            continue
        prompt_text = render_chat(
            tokenizer, prompt_messages, add_generation_prompt=True, tools=tools
        )
        if prompt_text:
            # Tool/reference context rides along as JSON strings: Arrow does
            # not have to unify heterogeneous schema dicts across records, and
            # TRL hands the columns back to the reward function per sample.
            prompts.append(
                {
                    "prompt": prompt_text,
                    "tools_json": json.dumps(tools) if tools else "",
                    "ref_calls_json": json.dumps(ref_calls) if ref_calls else "",
                }
            )

    if skipped_no_gold:
        logger.warning(
            "GRPO: skipped %d record(s) whose final assistant turn has no text content "
            "and no tool calls",
            skipped_no_gold,
        )
    if tool_records:
        logger.info(
            "GRPO: %d tool-call record(s) will use the verifiable tool-call reward",
            tool_records,
        )

    return Dataset.from_list(prompts) if prompts else dataset


def _upload_adapter(adapter_dir: Path, s3_prefix: str, s3, bucket: str) -> int:
    """Upload adapter directory to S3, return total size in bytes."""
    total_size = 0

    for file_path in adapter_dir.rglob("*"):
        if file_path.is_file():
            s3_key = s3_prefix + file_path.relative_to(adapter_dir).as_posix()
            s3.upload_file(str(file_path), bucket, s3_key)
            total_size += file_path.stat().st_size

    logger.info("Uploaded adapter: %s (%d bytes)", s3_prefix, total_size)
    return total_size


async def _get_project_id(db, job_id: str) -> str:
    """Get the project_id for a training job."""
    row = await db.fetchrow("SELECT project_id FROM training_jobs WHERE id = $1", job_id)
    if row is None:
        raise ValueError(f"Training job not found: {job_id}")
    return str(row["project_id"])


def _sum_gpu_runtime_seconds(metrics: dict) -> float:
    """Total GPU runtime across all training phases, in seconds.

    Sums every ``*runtime`` metric — the SFT runtime (``train_runtime`` /
    ``<phase>_train_runtime`` / ``iter_N_train_runtime``) AND the DPO/GRPO phase
    runtimes, which the aligned/reasoning strategies nest one level deep under a
    ``"dpo"``/``"grpo"`` key. Matching only ``"train_runtime"`` previously missed
    the DPO/GRPO GPU time, so aligned/reasoning runs were billed for the SFT pass
    only — undercharging the tenant. Recurses into nested metric dicts so both
    the flat and nested shapes are covered. ``bool`` is excluded (it subclasses
    ``int``).
    """
    total = 0.0
    for key, value in metrics.items():
        if isinstance(value, dict):
            total += _sum_gpu_runtime_seconds(value)
        elif (
            key.endswith("runtime")
            and isinstance(value, int | float)
            and not isinstance(value, bool)
        ):
            total += value
    return total


def _extract_training_runtime_seconds(metrics: dict) -> int:
    return int(round(_sum_gpu_runtime_seconds(metrics)))


def _training_billing_event_id(job_id: str, outcome: str) -> uuid.UUID:
    return uuid.uuid5(uuid.NAMESPACE_URL, f"training-billing:{job_id}:{outcome}")


def _teacher_serving_billing_event_id(job_id: str, outcome: str) -> uuid.UUID:
    return uuid.uuid5(uuid.NAMESPACE_URL, f"teacher-serving-billing:{job_id}:{outcome}")


def teacher_serving_share(gpu_class: str | None, hyperparams: dict) -> float:
    """Fraction of a container's GPU cost that the resident teacher accounts for.

    Only an on-policy run has a teacher inside the training container; every other
    mode reaches its teacher through the tenant's own API key, or not at all.

    The split is by device count, which is exact only because every device in a
    class is the same type — the teacher holds all but one card and the student
    holds the last. A mixed-GPU class would need real per-device metering, and is
    why no such class is offered.
    """
    if hyperparams.get(DISTILL_METHOD_HYPERPARAM) != ON_POLICY_DISTILL_METHOD:
        return 0.0
    devices = GPU_DEVICE_COUNTS.get((gpu_class or "").lower(), GPU_DEFAULT_DEVICE_COUNT)
    if devices < 2:
        return 0.0
    return (devices - 1) / devices


def split_teacher_serving_cost(
    gpu_seconds: int, cost_usd: float, share: float
) -> tuple[tuple[int, float], tuple[int, float]]:
    """Divide one container's bill into (student, teacher) halves.

    The teacher's share is computed and the student's is the remainder, so the two
    rows always re-add to exactly what the container cost. Splitting both
    independently would let rounding invent or lose a cent.
    """
    if share <= 0.0:
        return (gpu_seconds, cost_usd), (0, 0.0)
    teacher_seconds = int(round(gpu_seconds * share))
    teacher_cost = round(cost_usd * share, 2)
    return (
        (gpu_seconds - teacher_seconds, round(cost_usd - teacher_cost, 2)),
        (teacher_seconds, teacher_cost),
    )


async def _append_training_billing_outbox(
    conn,
    *,
    tenant_id: str,
    job_id: str,
    outcome: str,
    gpu_seconds: int,
    cost_usd: float,
    metadata: dict,
    teacher_share: float = 0.0,
) -> None:
    """Append the outbox row(s) for one finished run.

    An on-policy run produces two: the student's training time and the teacher's
    serving time, because they answer to different budgets — the teacher's share
    is what the teacher-GPU spend cap counts. Both are written on the caller's
    connection, inside the caller's transaction with the job's terminal status, so
    a crash can never commit one without the other.
    """
    (student_seconds, student_cost), (teacher_seconds, teacher_cost) = split_teacher_serving_cost(
        gpu_seconds, cost_usd, teacher_share
    )

    await conn.execute(
        """INSERT INTO billing_outbox
        (id, tenant_id, operation, resource_id, tokens_in, tokens_out,
         gpu_seconds, cost_usd, metadata)
        VALUES ($1, $2, 'training', $3, 0, 0, $4, $5, $6::jsonb)
        ON CONFLICT (id) DO NOTHING""",
        _training_billing_event_id(job_id, outcome),
        tenant_id,
        uuid.UUID(job_id),
        student_seconds,
        student_cost,
        json.dumps(metadata),
    )

    if teacher_share <= 0.0:
        return

    await conn.execute(
        """INSERT INTO billing_outbox
        (id, tenant_id, operation, resource_id, tokens_in, tokens_out,
         gpu_seconds, cost_usd, metadata)
        VALUES ($1, $2, 'teacher_serving', $3, 0, 0, $4, $5, $6::jsonb)
        ON CONFLICT (id) DO NOTHING""",
        _teacher_serving_billing_event_id(job_id, outcome),
        tenant_id,
        uuid.UUID(job_id),
        teacher_seconds,
        teacher_cost,
        json.dumps({**metadata, "teacher_device_share": teacher_share}),
    )


async def _finalize_failed_training_billing(
    conn,
    job_id: str,
    settings,
    *,
    mode: str,
    method: str,
    base_model: str,
    hyperparams: dict | None = None,
) -> None:
    """Persist failed-job actual_cost and append the corresponding outbox row.

    This runs inside the same DB transaction as the FAILED status update so the
    job terminal state and billing ledger entry remain consistent.

    A failed on-policy run is split the same way a successful one is. Billing only
    successful runs against the teacher-GPU cap would leave a cheaper way to spend
    unbounded teacher time: a teacher that boots, holds its card, and then fails.
    """
    row = await conn.fetchrow(
        "SELECT tenant_id, started_at, completed_at, gpu_class FROM training_jobs WHERE id = $1",
        job_id,
    )
    if row is None or row["started_at"] is None or row["completed_at"] is None:
        return

    elapsed = (row["completed_at"] - row["started_at"]).total_seconds()
    gpu_seconds = int(round(elapsed))
    min_billable = getattr(settings, "min_billable_seconds", 300)
    gpu_class = (row["gpu_class"] or "").lower()
    rate = GPU_HOURLY_RATES.get(gpu_class, GPU_DEFAULT_HOURLY_RATE)

    if elapsed < min_billable:
        actual_cost = 0.0
        logger.info(
            "Voided billing for job %s (ran %.1fs < %ds threshold)",
            job_id,
            elapsed,
            min_billable,
        )
    else:
        elapsed_hours = elapsed / 3600.0
        actual_cost = round(elapsed_hours * rate, 2)
        logger.info(
            "Billed failed job %s for actual GPU time: %.1fs = $%.2f (%s @ $%.2f/hr)",
            job_id,
            elapsed,
            actual_cost,
            gpu_class or "default",
            rate,
        )

    await conn.execute(
        "UPDATE training_jobs SET actual_cost = $2 WHERE id = $1",
        job_id,
        actual_cost,
    )
    await _append_training_billing_outbox(
        conn,
        tenant_id=str(row["tenant_id"]),
        job_id=job_id,
        outcome="failed",
        gpu_seconds=gpu_seconds,
        cost_usd=actual_cost,
        teacher_share=teacher_serving_share(gpu_class, hyperparams or {}),
        metadata={
            "status": "failed",
            "mode": mode,
            "method": method,
            "base_model": base_model,
            "gpu_class": gpu_class or None,
            "min_billable_seconds": min_billable,
        },
    )


def _is_bf16_supported() -> bool:
    """Check if the GPU supports bf16."""
    try:
        import torch

        if torch.cuda.is_available():
            capability = torch.cuda.get_device_capability()
            return capability[0] >= 8
    except ImportError:
        pass
    return False
