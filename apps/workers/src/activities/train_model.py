"""Training activities — runs fine-tuning jobs via pluggable strategies.

Strategy-based modes (dispatched by start_training activity):
  - quick:     SFT only (fastest iteration)
  - aligned:   SFT → DPO (production quality alignment)
  - reasoning: SFT → GRPO (reward-guided reasoning optimization)

Workflow-based activities (called by TrainIterativeWorkflow):
  - train_sft_round:    Single SFT iteration with checkpoint save
  - evaluate_holdout:   Validation eval for early stopping decisions

Uses TrainingEngine protocol (default: Unsloth) for model loading,
LLMJudge protocol for scoring, and Redis streams for real-time metrics.
"""

import json
import logging
import tempfile
import time
import uuid
from datetime import UTC, datetime
from pathlib import Path

from temporalio import activity
from temporalio.exceptions import ApplicationError

from src import s3_paths
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
from src.constants import GPU_DEFAULT_HOURLY_RATE, GPU_HOURLY_RATES, TrainingJobStatus
from src.gpu_provider import GpuProvider
from src.infra import InfraContainer
from src.notifications import EVENT_TRAINING_COMPLETE, enqueue_notification
from src.tenant_config import TenantLlmConfig

logger = logging.getLogger("platform.training")


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
            """SELECT j.status, j.metrics, m.adapter_path, m.adapter_size_bytes
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

                    await conn.execute(
                        """INSERT INTO models
                        (tenant_id, project_id, training_job_id, name, base_model,
                         adapter_path, adapter_size_bytes, version)
                        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)""",
                        input.tenant_id,
                        project_id,
                        job_id,
                        model_name,
                        input.base_model,
                        result.adapter_path,
                        result.adapter_size_bytes,
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
    async def run(self, input: FinalizeIterativeTrainingInput) -> None:
        db = self.infra.db
        job_id = input.training_job_id

        # Calculate actual cost from aggregate iteration runtimes
        gpu_rate = GPU_HOURLY_RATES.get(input.gpu_class or "", GPU_DEFAULT_HOURLY_RATE)
        total_runtime = 0.0
        for val in input.metrics.values():
            if isinstance(val, dict):
                for rk, rv in val.items():
                    if rk.endswith("train_runtime") and isinstance(rv, (int, float)):
                        total_runtime += rv
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

                await conn.execute(
                    """INSERT INTO models
                    (tenant_id, project_id, training_job_id, name, base_model,
                     adapter_path, adapter_size_bytes, version)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)""",
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
    engine = get_engine(settings)
    hp = input.hyperparams
    job_id = input.training_job_id

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

        strategy = get_strategy(input.mode)
        metrics = strategy.execute(
            model=model,
            tokenizer=tokenizer,
            dataset=dataset,
            hp=hp,
            job_id=job_id,
            max_seq_length=max_seq_length,
            tenant_id=input.tenant_id,
            dataset_path=input.dataset_path,
            s3=s3,
            bucket=s3_bucket,
            llm_config=llm_config,
            settings=settings,
        )

        gpu_rate = GPU_HOURLY_RATES.get(input.gpu_class or "", GPU_DEFAULT_HOURLY_RATE)
        total_runtime = sum(
            v
            for k, v in metrics.items()
            if k.endswith("train_runtime") and isinstance(v, (int, float))
        )
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

        try:
            activity.heartbeat(f"eval_iter_{iteration}_running")
        except Exception:
            pass

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

    save_steps = hp.get("save_steps", 100)
    enable_checkpoints = tenant_id is not None
    callbacks = [callback]
    if enable_checkpoints:
        CkptClass = _build_checkpoint_callback_class(tenant_id, job_id, s3, bucket)
        callbacks.append(CkptClass())

    training_args = SFTConfig(
        output_dir=f"/tmp/sft-{job_id[:8]}",
        per_device_train_batch_size=hp.get("per_device_train_batch_size", 2),
        gradient_accumulation_steps=hp.get("gradient_accumulation_steps", 4),
        num_train_epochs=hp.get("num_train_epochs", 3),
        warmup_steps=hp.get("warmup_steps", 10),
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

    def reasoning_reward(completions: list[str], **kwargs) -> list[float]:
        return [judge.score_reasoning(c) for c in completions]

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

            try:
                activity.heartbeat(f"step={current_step}/{max_steps}")
            except Exception:
                pass

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
                rows.append({"messages": record.get("messages", [])})

    return Dataset.from_list(rows)


def _render_sft_dataset(dataset, tokenizer):
    """Render a messages-form dataset into `{"text": ...}` for SFT / eval.

    Uses the model's own chat template so the full supervised example (prompt +
    assistant turn + the template's EOS) matches serve-time formatting.
    """
    from src.activities.chat_template import render_chat

    def _map(row):
        return {"text": render_chat(tokenizer, row["messages"], add_generation_prompt=False)}

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

    messages_list = dataset["messages"]
    for messages in messages_list:
        if max_pairs is not None and len(chosen_texts) >= max_pairs:
            break

        # Recover the prompt turns and gold answer from the structured messages
        # (never by string-splitting a formatted string — that only worked for
        # ChatML and broke on every other model's template).
        prompt_messages, gold = split_prompt_and_response(messages)
        if prompt_messages is None:
            continue

        # Format the prompt with the model's own template so the sampled
        # ("rejected") response is generated exactly as the model is served.
        gen_prompt = render_chat(tokenizer, prompt_messages, add_generation_prompt=True)
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
            render_chat(tokenizer, [*prompt_messages, {"role": "assistant", "content": gold}])
        )
        rejected_texts.append(
            render_chat(tokenizer, [*prompt_messages, {"role": "assistant", "content": sampled}])
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
    for messages in dataset["messages"]:
        prompt_messages, _gold = split_prompt_and_response(messages)
        # Fall back to the whole conversation when there is no trailing assistant
        # turn (e.g. prompt-only GRPO datasets).
        if prompt_messages is None:
            prompt_messages = messages
        if not prompt_messages:
            continue
        prompt_text = render_chat(tokenizer, prompt_messages, add_generation_prompt=True)
        if prompt_text:
            prompts.append({"prompt": prompt_text})

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


def _extract_training_runtime_seconds(metrics: dict) -> int:
    total_runtime = sum(
        v for k, v in metrics.items() if k.endswith("train_runtime") and isinstance(v, (int, float))
    )
    return int(round(total_runtime))


def _training_billing_event_id(job_id: str, outcome: str) -> uuid.UUID:
    return uuid.uuid5(uuid.NAMESPACE_URL, f"training-billing:{job_id}:{outcome}")


async def _append_training_billing_outbox(
    conn,
    *,
    tenant_id: str,
    job_id: str,
    outcome: str,
    gpu_seconds: int,
    cost_usd: float,
    metadata: dict,
) -> None:
    await conn.execute(
        """INSERT INTO billing_outbox
        (id, tenant_id, operation, resource_id, tokens_in, tokens_out,
         gpu_seconds, cost_usd, metadata)
        VALUES ($1, $2, 'training', $3, 0, 0, $4, $5, $6::jsonb)
        ON CONFLICT (id) DO NOTHING""",
        _training_billing_event_id(job_id, outcome),
        tenant_id,
        uuid.UUID(job_id),
        gpu_seconds,
        cost_usd,
        json.dumps(metadata),
    )


async def _finalize_failed_training_billing(
    conn,
    job_id: str,
    settings,
    *,
    mode: str,
    method: str,
    base_model: str,
) -> None:
    """Persist failed-job actual_cost and append the corresponding outbox row.

    This runs inside the same DB transaction as the FAILED status update so the
    job terminal state and billing ledger entry remain consistent.
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
