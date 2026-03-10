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
from datetime import UTC, datetime
from pathlib import Path

from temporalio import activity

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
from src.constants import TrainingJobStatus
from src.gpu_provider import GpuProvider
from src.infra import InfraContainer

logger = logging.getLogger("platform.training")


class StartTrainingActivity:
    def __init__(self, infra: InfraContainer, gpu_provider: GpuProvider | None = None):
        self.infra = infra
        self.gpu_provider = gpu_provider

    @activity.defn(name="start_training")
    async def run(self, input: StartTrainingInput) -> StartTrainingOutput:
        """Run a fine-tuning job. Called by TrainWorkflow.

        Dispatches to the configured GpuProvider (local or Modal).
        Falls back to direct local execution if no provider is set.
        """
        db = self.infra.db
        job_id = input.training_job_id

        try:
            await db.execute(
                "UPDATE training_jobs SET status = $1, started_at = NOW() WHERE id = $2",
                TrainingJobStatus.TRAINING,
                job_id,
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
                )
                result = StartTrainingOutput(
                    adapter_path=result_dict["adapter_path"],
                    adapter_size_bytes=result_dict["adapter_size_bytes"],
                    metrics=result_dict["metrics"],
                )
            else:
                result = await _run_training(input, self.infra)

            await db.execute(
                """UPDATE training_jobs
                SET status = $1,
                    metrics = $3,
                    actual_cost = $4,
                    completed_at = NOW()
                WHERE id = $2""",
                TrainingJobStatus.COMPLETED,
                job_id,
                json.dumps(result.metrics),
                result.metrics.get("estimated_cost"),
            )

            model_name = f"{input.base_model.split('/')[-1]}-{input.mode}-{job_id[:8]}"
            project_id = await _get_project_id(db, job_id)

            # Auto-increment version for the same base_model within this project
            max_version = await db.fetchval(
                """SELECT COALESCE(MAX(version), 0) FROM models
                WHERE project_id = $1 AND tenant_id = $2 AND base_model = $3""",
                project_id,
                input.tenant_id,
                input.base_model,
            )
            next_version = (max_version or 0) + 1

            await db.execute(
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

            logger.info("Training completed for job %s, model: %s", job_id, model_name)
            return result

        except Exception as e:
            logger.exception("Training failed for job %s", job_id)
            await db.execute(
                """UPDATE training_jobs
                SET status = $1, error_message = $3, completed_at = NOW()
                WHERE id = $2""",
                TrainingJobStatus.FAILED,
                job_id,
                str(e)[:2000],
            )
            raise


class TrainSftRoundActivity:
    """Single SFT iteration for the iterative workflow.

    Each round: load model (+adapter if continuing), train one SFT pass,
    save adapter checkpoint to S3. The loop lives in TrainIterativeWorkflow.
    """

    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="train_sft_round")
    async def run(self, input: TrainSftRoundInput) -> TrainSftRoundOutput:
        engine = get_engine(self.infra.settings)
        hp = input.hyperparams
        job_id = input.training_job_id
        iteration = input.iteration

        _get_metrics_collector(self.infra.settings)

        # Mark training as started on the first iteration
        if iteration == 0:
            await self.infra.db.execute(
                "UPDATE training_jobs SET status = $1, started_at = NOW() WHERE id = $2",
                TrainingJobStatus.TRAINING,
                job_id,
            )

        with tempfile.TemporaryDirectory(prefix=f"sft-round-{job_id[:8]}-") as tmpdir:
            tmpdir_path = Path(tmpdir)

            # Download dataset
            dataset_local = tmpdir_path / "dataset.jsonl"
            _download_dataset(
                input.dataset_path,
                dataset_local,
                self.infra.s3,
                self.infra.s3_bucket,
            )
            dataset = _load_chatml_dataset(dataset_local)

            # Load model
            load_in_4bit = input.method == "qlora"
            max_seq_length = hp.get("max_seq_length", 2048)
            model, tokenizer = engine.load_model(
                model_name=input.base_model,
                max_seq_length=max_seq_length,
                load_in_4bit=load_in_4bit,
            )

            # Attach adapter
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

            # If resuming from previous iteration, load adapter weights
            if input.adapter_path:
                prev_adapter_dir = tmpdir_path / "prev_adapter"
                prev_adapter_dir.mkdir(parents=True)
                _download_adapter(
                    input.adapter_path,
                    prev_adapter_dir,
                    self.infra.s3,
                    self.infra.s3_bucket,
                )
                model.load_adapter(
                    str(prev_adapter_dir),
                    adapter_name="default",
                )
                logger.info("Loaded adapter from previous iteration: %s", input.adapter_path)

            # Train one SFT round
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
                s3=self.infra.s3,
                bucket=self.infra.s3_bucket,
            )

            # Save adapter checkpoint
            adapter_dir = tmpdir_path / "adapter"
            engine.save_adapter(model, tokenizer, adapter_dir)

            ckpt_s3_path = (
                s3_paths.checkpoint_prefix(input.tenant_id, job_id) + f"iter-{iteration}/"
            )
            adapter_size = _upload_adapter(
                adapter_dir,
                ckpt_s3_path,
                self.infra.s3,
                self.infra.s3_bucket,
            )

            logger.info(
                "Iteration %d complete for job %s, checkpoint: %s",
                iteration,
                job_id,
                ckpt_s3_path,
            )

            return TrainSftRoundOutput(
                adapter_path=ckpt_s3_path,
                adapter_size_bytes=adapter_size,
                metrics=metrics,
            )


class EvaluateHoldoutActivity:
    """Run holdout validation after an SFT round.

    Loads the adapter from the iteration checkpoint and evaluates
    on the validation split. Returns eval_loss for early stopping decisions.
    Streams progress metrics to Redis for real-time UI visibility.
    """

    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="evaluate_holdout")
    async def run(self, input: EvaluateHoldoutInput) -> EvaluateHoldoutOutput:
        engine = get_engine(self.infra.settings)
        hp = input.hyperparams
        job_id = input.training_job_id
        iteration = input.iteration

        _get_sync_redis(self.infra.settings)

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
            _download_dataset(val_s3_path, val_local, self.infra.s3, self.infra.s3_bucket)
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

            # Load model + adapter from this iteration's checkpoint
            max_seq_length = hp.get("max_seq_length", 2048)
            load_in_4bit = input.method == "qlora"
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

            # Load the adapter weights from this iteration
            adapter_dir = tmpdir_path / "adapter"
            adapter_dir.mkdir(parents=True)
            _download_adapter(input.adapter_path, adapter_dir, self.infra.s3, self.infra.s3_bucket)
            model.load_adapter(str(adapter_dir), adapter_name="default")

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
                "Holdout eval iteration %d for job %s: eval_loss=%.4f",
                iteration,
                job_id,
                eval_loss,
            )

            return EvaluateHoldoutOutput(
                eval_loss=eval_loss,
                metrics={
                    "iteration": iteration,
                    "eval_loss": eval_loss,
                },
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
        gpu_rates = {
            "t4": 0.80,
            "a10g": 1.20,
            "l40s": 1.80,
            "a10040gb": 2.00,
            "a10080gb": 3.00,
            "h100": 4.50,
        }
        gpu_rate = gpu_rates.get(input.gpu_class or "", 0.80)
        total_runtime = 0.0
        for val in input.metrics.values():
            if isinstance(val, dict):
                for rk, rv in val.items():
                    if rk.endswith("train_runtime") and isinstance(rv, (int, float)):
                        total_runtime += rv
        runtime_hours = total_runtime / 3600.0
        actual_cost = round(runtime_hours * gpu_rate, 2)
        input.metrics["estimated_cost"] = actual_cost

        await db.execute(
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

        model_name = f"{input.base_model.split('/')[-1]}-{input.mode}-{job_id[:8]}"
        project_id = await _get_project_id(db, job_id)

        max_version = await db.fetchval(
            """SELECT COALESCE(MAX(version), 0) FROM models
            WHERE project_id = $1 AND tenant_id = $2 AND base_model = $3""",
            project_id,
            input.tenant_id,
            input.base_model,
        )
        next_version = (max_version or 0) + 1

        await db.execute(
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


async def _run_training(input: StartTrainingInput, infra: InfraContainer) -> StartTrainingOutput:
    """Load model via engine, dispatch to strategy, upload adapter."""
    engine = get_engine(infra.settings)
    hp = input.hyperparams
    job_id = input.training_job_id

    # Ensure MetricsCollector is initialized before any training callbacks use it
    _get_metrics_collector(infra.settings)

    with tempfile.TemporaryDirectory(prefix=f"train-{job_id[:8]}-") as tmpdir:
        tmpdir_path = Path(tmpdir)

        # Download dataset from S3
        dataset_local = tmpdir_path / "dataset.jsonl"
        _download_dataset(input.dataset_path, dataset_local, infra.s3, infra.s3_bucket)

        dataset = _load_chatml_dataset(dataset_local)
        logger.info("Loaded dataset: %d examples", len(dataset))

        # Load model via engine protocol
        load_in_4bit = input.method == "qlora"
        max_seq_length = hp.get("max_seq_length", 2048)

        model, tokenizer = engine.load_model(
            model_name=input.base_model,
            max_seq_length=max_seq_length,
            load_in_4bit=load_in_4bit,
        )

        # Attach LoRA adapters via engine protocol
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

        # Resolve per-tenant LLM config for judge (DPO/GRPO need an LLM judge)
        from src.tenant_config import get_tenant_llm_config

        llm_config = await get_tenant_llm_config(
            db=infra.db,
            tenant_id=input.tenant_id,
            default_api_base_url=infra.settings.llm_api_base_url,
            default_api_key=infra.settings.llm_api_key,
            default_model=infra.settings.llm_model,
        )

        # Dispatch to registered strategy
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
            s3=infra.s3,
            bucket=infra.s3_bucket,
            llm_config=llm_config,
        )

        # Calculate actual cost from runtime
        gpu_rates = {
            "t4": 0.80,
            "a10g": 1.20,
            "l40s": 1.80,
            "a10040gb": 2.00,
            "a10080gb": 3.00,
            "h100": 4.50,
        }
        gpu_rate = gpu_rates.get(input.gpu_class or "", 0.80)
        total_runtime = sum(
            v
            for k, v in metrics.items()
            if k.endswith("train_runtime") and isinstance(v, (int, float))
        )
        runtime_hours = total_runtime / 3600.0
        metrics["estimated_cost"] = round(runtime_hours * gpu_rate, 2)

        # Save adapter via engine protocol
        adapter_dir = tmpdir_path / "adapter"
        engine.save_adapter(model, tokenizer, adapter_dir)

        # Upload adapter to S3
        adapter_s3_path = s3_paths.adapter_training_prefix(input.tenant_id, job_id)
        adapter_size = _upload_adapter(adapter_dir, adapter_s3_path, infra.s3, infra.s3_bucket)

        return StartTrainingOutput(
            adapter_path=adapter_s3_path,
            adapter_size_bytes=adapter_size,
            metrics=metrics,
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
            model, tokenizer, dataset, hp, job_id, max_seq_length, llm_config=llm_config
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
            model, tokenizer, dataset, hp, job_id, max_seq_length, llm_config=llm_config
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


def _train_dpo(model, tokenizer, dataset, hp, job_id, max_seq_length, llm_config):
    """Run DPO (Direct Preference Optimization) training."""
    from trl import DPOConfig, DPOTrainer

    CallbackClass = _build_callback_class()
    callback = CallbackClass(job_id, phase="dpo")

    dpo_dataset = _create_dpo_pairs(dataset, tokenizer, llm_config)

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


def _train_grpo(model, tokenizer, dataset, hp, job_id, max_seq_length, llm_config):
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

    eval_args = SFTConfig(
        output_dir="/tmp/eval-holdout",
        per_device_eval_batch_size=hp.get("per_device_train_batch_size", 2),
        max_seq_length=max_seq_length,
        fp16=not _is_bf16_supported(),
        bf16=_is_bf16_supported(),
        report_to="none",
    )

    trainer = SFTTrainer(
        model=model,
        processing_class=tokenizer,
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

        def on_log(self, args, state, control, logs=None, **kwargs):
            if logs is None:
                return

            metrics = {
                "step": str(state.global_step),
                "epoch": str(round(state.epoch or 0, 2)),
                "loss": str(logs.get("loss", 0)),
                "learning_rate": str(logs.get("learning_rate", 0)),
                "grad_norm": str(logs.get("grad_norm", 0)),
                "phase": self.phase,
                "timestamp": datetime.now(UTC).isoformat(),
            }

            gpu = _get_gpu_metrics()
            if gpu:
                metrics.update(gpu)

            try:
                stream_key = f"training:metrics:{self.job_id}"
                _get_metrics_collector().record(stream_key, metrics, maxlen=10000)
            except Exception as e:
                logger.warning("Failed to stream metrics: %s", e)

            try:
                activity.heartbeat(f"step={state.global_step}")
            except Exception:
                pass

        def on_train_begin(self, args, state, control, **kwargs):
            _stream_metric(
                self.job_id,
                {
                    "event": "train_begin",
                    "phase": self.phase,
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
    """Load a ChatML JSONL dataset into HuggingFace Dataset format."""
    from datasets import Dataset

    records = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                records.append(json.loads(line))

    texts = []
    for record in records:
        messages = record.get("messages", [])
        text = _format_chatml(messages)
        texts.append({"text": text})

    return Dataset.from_list(texts)


def _format_chatml(messages: list[dict]) -> str:
    """Format messages as ChatML text."""
    parts = []
    for msg in messages:
        role = msg.get("role", "user")
        content = msg.get("content", "")
        parts.append(f"<|im_start|>{role}\n{content}<|im_end|>")
    return "\n".join(parts)


def _create_dpo_pairs(dataset, tokenizer, llm_config):
    """Create DPO preference pairs using per-tenant LLM judge."""
    from datasets import Dataset

    judge = OpenAICompatibleJudge(
        api_base=llm_config.api_base_url,
        api_key=llm_config.api_key,
        model=llm_config.model,
    )
    chosen_texts = []
    rejected_texts = []

    for text in dataset["text"]:
        parts = text.split("<|im_start|>assistant\n")
        if len(parts) <= 1:
            continue

        prompt_part = "<|im_start|>assistant\n".join(parts[:-1])
        original_response = parts[-1].split("<|im_end|>")[0]

        truncated = original_response[: max(10, len(original_response) // 3)]

        original_score = judge.score_response(prompt_part, original_response)
        degraded_score = judge.score_response(prompt_part, truncated)

        original_full = prompt_part + "<|im_start|>assistant\n" + original_response + "<|im_end|>"
        degraded_full = prompt_part + "<|im_start|>assistant\n" + truncated + "<|im_end|>"

        if original_score >= degraded_score:
            chosen_texts.append(original_full)
            rejected_texts.append(degraded_full)
        else:
            chosen_texts.append(degraded_full)
            rejected_texts.append(original_full)

    if not chosen_texts:
        for text in dataset["text"]:
            chosen_texts.append(text)
            rejected_texts.append(text[: len(text) // 2])

    return Dataset.from_dict({"chosen": chosen_texts, "rejected": rejected_texts})


def _create_grpo_prompts(dataset, tokenizer):
    """Extract prompts from dataset for GRPO training."""
    from datasets import Dataset

    prompts = []
    for text in dataset["text"]:
        parts = text.split("<|im_start|>user\n")
        for part in parts[1:]:
            user_msg = part.split("<|im_end|>")[0].strip()
            if user_msg:
                prompts.append({"prompt": user_msg})

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
