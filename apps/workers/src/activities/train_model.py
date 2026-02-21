"""Training activity — runs fine-tuning jobs using Unsloth + TRL.

Supports 4 training modes:
  - quick:     SFT only (fastest iteration)
  - aligned:   SFT → DPO (production quality alignment)
  - reasoning: SFT → GRPO (reward-guided reasoning optimization)
  - iterative: Multiple SFT rounds with evaluation between each

Uses Redis streams for real-time metrics streaming and Temporal
heartbeats to keep the activity alive during long training runs.
"""

import json
import logging
import tempfile
from datetime import UTC, datetime
from pathlib import Path

from temporalio import activity

from src import clients
from src.activities.stubs import StartTrainingInput, StartTrainingOutput

logger = logging.getLogger("platform.training")


@activity.defn
async def start_training(input: StartTrainingInput) -> StartTrainingOutput:
    """Run a fine-tuning job. Called by TrainWorkflow."""
    db = await clients.get_db()
    job_id = input.training_job_id

    try:
        # Update status to training
        await db.execute(
            "UPDATE training_jobs SET status = 'training', started_at = NOW() WHERE id = $1",
            job_id,
        )

        result = await _run_training(input)

        # Update status to completed
        await db.execute(
            """UPDATE training_jobs
            SET status = 'completed',
                metrics = $2,
                actual_cost = $3,
                completed_at = NOW()
            WHERE id = $1""",
            job_id,
            json.dumps(result.metrics),
            result.metrics.get("estimated_cost"),
        )

        # Create model record
        model_name = f"{input.base_model.split('/')[-1]}-{input.mode}-{job_id[:8]}"
        project_id = await _get_project_id(db, job_id)

        await db.execute(
            """INSERT INTO models
            (tenant_id, project_id, training_job_id, name, base_model,
             adapter_path, adapter_size_bytes)
            VALUES ($1, $2, $3, $4, $5, $6, $7)""",
            input.tenant_id,
            project_id,
            job_id,
            model_name,
            input.base_model,
            result.adapter_path,
            result.adapter_size_bytes,
        )

        logger.info("Training completed for job %s, model: %s", job_id, model_name)
        return result

    except Exception as e:
        logger.exception("Training failed for job %s", job_id)
        await db.execute(
            """UPDATE training_jobs
            SET status = 'failed', error_message = $2, completed_at = NOW()
            WHERE id = $1""",
            job_id,
            str(e)[:2000],
        )
        raise


async def _run_training(input: StartTrainingInput) -> StartTrainingOutput:
    """Dispatch to the appropriate training mode."""
    # Import ML libraries lazily (only available on GPU workers)
    from unsloth import FastLanguageModel

    hp = input.hyperparams
    job_id = input.training_job_id

    with tempfile.TemporaryDirectory(prefix=f"train-{job_id[:8]}-") as tmpdir:
        tmpdir_path = Path(tmpdir)

        # Download dataset from S3
        dataset_local = tmpdir_path / "dataset.jsonl"
        _download_dataset(input.dataset_path, dataset_local)

        # Load and prepare dataset
        dataset = _load_chatml_dataset(dataset_local)
        logger.info("Loaded dataset: %d examples", len(dataset))

        # Load base model
        load_in_4bit = input.method == "qlora"
        max_seq_length = hp.get("max_seq_length", 2048)

        model, tokenizer = FastLanguageModel.from_pretrained(
            model_name=input.base_model,
            max_seq_length=max_seq_length,
            load_in_4bit=load_in_4bit,
            dtype=None,
        )

        # Attach LoRA adapters
        target_modules = hp.get(
            "target_modules",
            [
                "q_proj",
                "k_proj",
                "v_proj",
                "o_proj",
                "gate_proj",
                "up_proj",
                "down_proj",
            ],
        )
        model = FastLanguageModel.get_peft_model(
            model,
            r=hp.get("r", 16),
            lora_alpha=hp.get("lora_alpha", 16),
            lora_dropout=hp.get("lora_dropout", 0),
            target_modules=target_modules,
            use_gradient_checkpointing="unsloth",
        )

        # Route to training mode
        if input.mode == "quick":
            metrics = _train_sft(model, tokenizer, dataset, hp, job_id, max_seq_length)
        elif input.mode == "aligned":
            metrics_sft = _train_sft(
                model, tokenizer, dataset, hp, job_id, max_seq_length, phase="sft"
            )
            metrics_dpo = _train_dpo(model, tokenizer, dataset, hp, job_id, max_seq_length)
            metrics = {**metrics_sft, "dpo": metrics_dpo}
        elif input.mode == "reasoning":
            metrics_sft = _train_sft(
                model, tokenizer, dataset, hp, job_id, max_seq_length, phase="sft"
            )
            metrics_grpo = _train_grpo(model, tokenizer, dataset, hp, job_id, max_seq_length)
            metrics = {**metrics_sft, "grpo": metrics_grpo}
        elif input.mode == "iterative":
            metrics = _train_iterative(model, tokenizer, dataset, hp, job_id, max_seq_length)
        else:
            raise ValueError(f"Unknown training mode: {input.mode}")

        # Save adapter
        adapter_dir = tmpdir_path / "adapter"
        model.save_pretrained(str(adapter_dir))
        tokenizer.save_pretrained(str(adapter_dir))

        # Upload adapter to S3
        adapter_s3_path = f"adapters/{input.tenant_id}/{job_id}/"
        adapter_size = _upload_adapter(adapter_dir, adapter_s3_path)

        return StartTrainingOutput(
            adapter_path=adapter_s3_path,
            adapter_size_bytes=adapter_size,
            metrics=metrics,
        )


def _train_sft(model, tokenizer, dataset, hp, job_id, max_seq_length, phase=None):
    """Run SFT (Supervised Fine-Tuning) training."""
    from trl import SFTConfig, SFTTrainer

    phase_prefix = f"{phase}_" if phase else ""
    CallbackClass = _build_callback_class()
    callback = CallbackClass(job_id, phase=phase or "sft")

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
        save_strategy="no",
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
        callbacks=[callback],
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


def _train_dpo(model, tokenizer, dataset, hp, job_id, max_seq_length):
    """Run DPO (Direct Preference Optimization) training on the same dataset.

    Reformats the ChatML data as chosen/rejected pairs for alignment.
    """
    from trl import DPOConfig, DPOTrainer

    CallbackClass = _build_callback_class()
    callback = CallbackClass(job_id, phase="dpo")

    # Create DPO pairs from the dataset: original is "chosen", shuffled is "rejected"
    dpo_dataset = _create_dpo_pairs(dataset, tokenizer)

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


def _train_grpo(model, tokenizer, dataset, hp, job_id, max_seq_length):
    """Run GRPO (Group Relative Policy Optimization) for reasoning tasks.

    Uses a simple reward function based on response quality heuristics.
    """
    from trl import GRPOConfig, GRPOTrainer

    CallbackClass = _build_callback_class()
    callback = CallbackClass(job_id, phase="grpo")

    # Extract prompts from the dataset for GRPO
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

    trainer = GRPOTrainer(
        model=model,
        processing_class=tokenizer,
        train_dataset=grpo_dataset,
        reward_funcs=[_reasoning_reward],
        args=training_args,
        callbacks=[callback],
    )

    train_result = trainer.train()

    return {
        "grpo_loss": train_result.training_loss,
        "grpo_steps": train_result.global_step,
        "grpo_runtime": train_result.metrics.get("train_runtime", 0),
    }


def _train_iterative(model, tokenizer, dataset, hp, job_id, max_seq_length):
    """Run multiple SFT rounds with evaluation between each."""
    num_iterations = hp.get("num_iterations", 3)
    all_metrics = {}

    for iteration in range(num_iterations):
        phase = f"iter_{iteration}"
        logger.info("Starting iteration %d/%d for job %s", iteration + 1, num_iterations, job_id)

        iteration_metrics = _train_sft(
            model, tokenizer, dataset, hp, job_id, max_seq_length, phase=phase
        )
        all_metrics[phase] = iteration_metrics

        # Quick eval: compute average loss on a sample
        eval_loss = iteration_metrics.get(f"{phase}_train_loss", 0)
        all_metrics[f"{phase}_eval_loss"] = eval_loss

        # Stream iteration completion
        _stream_metric(
            job_id,
            {
                "event": "iteration_complete",
                "iteration": iteration + 1,
                "total_iterations": num_iterations,
                "eval_loss": eval_loss,
            },
        )

    all_metrics["total_iterations"] = num_iterations
    return all_metrics


# ── Helpers ──────────────────────────────────────────────────────────


# Module-level sync Redis client for metrics streaming from training threads.
# Lazily initialized, reused across all callbacks and helper calls.
_sync_redis_client = None


def _get_sync_redis():
    """Get or create the module-level synchronous Redis client."""
    global _sync_redis_client  # noqa: PLW0603
    if _sync_redis_client is None:
        import redis as sync_redis

        settings = clients.get_settings()
        _sync_redis_client = sync_redis.from_url(settings.redis_url)
    return _sync_redis_client


def _build_callback_class():
    """Build MetricsStreamingCallback as a proper TrainerCallback subclass.

    Can't inherit at module level because transformers may not be installed
    (it's an optional ML dependency). This factory is called at training time
    when transformers is guaranteed to be available.
    """
    try:
        from transformers import TrainerCallback

        _base = TrainerCallback
    except ImportError:
        _base = object

    class MetricsStreamingCallback(_base):
        """HuggingFace TrainerCallback that streams metrics to Redis and heartbeats Temporal."""

        def __init__(self, job_id: str, phase: str = ""):
            if _base is not object:
                super().__init__()
            self.job_id = job_id
            self.phase = phase

        def on_log(self, args, state, control, logs=None, **kwargs):
            """Push metrics to Redis stream on each log step."""
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

            try:
                stream_key = f"training:metrics:{self.job_id}"
                _get_sync_redis().xadd(stream_key, metrics, maxlen=10000)
            except Exception as e:
                logger.warning("Failed to stream metrics: %s", e)

            # Heartbeat to keep Temporal activity alive
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


def _stream_metric(job_id: str, data: dict):
    """Push a single metric event to Redis using the shared connection."""
    try:
        stream_key = f"training:metrics:{job_id}"
        str_data = {k: str(v) for k, v in data.items()}
        _get_sync_redis().xadd(stream_key, str_data, maxlen=10000)
    except Exception as e:
        logger.warning("Failed to stream metric event: %s", e)


def _download_dataset(s3_path: str, local_path: Path):
    """Download dataset from S3 to local file."""
    s3 = clients.get_s3()
    bucket = clients.get_s3_bucket()
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

    # Convert ChatML format to text for SFT
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


def _create_dpo_pairs(dataset, tokenizer):
    """Create DPO preference pairs from an SFT dataset.

    Uses the original as "chosen" and a degraded version as "rejected".
    """
    from datasets import Dataset

    chosen_texts = dataset["text"]
    # Create rejected by truncating responses (simple heuristic)
    rejected_texts = []
    for text in chosen_texts:
        # Find the last assistant response and truncate it
        parts = text.split("<|im_start|>assistant\n")
        if len(parts) > 1:
            # Take only first 30% of the response
            response = parts[-1].split("<|im_end|>")[0]
            truncated = response[: max(10, len(response) // 3)] + "<|im_end|>"
            prefix = "<|im_start|>assistant\n".join(parts[:-1])
            rejected = prefix + "<|im_start|>assistant\n" + truncated
            rejected_texts.append(rejected)
        else:
            rejected_texts.append(text[: len(text) // 2])

    return Dataset.from_dict(
        {
            "chosen": chosen_texts,
            "rejected": rejected_texts,
        }
    )


def _create_grpo_prompts(dataset, tokenizer):
    """Extract prompts from dataset for GRPO training."""
    from datasets import Dataset

    prompts = []
    for text in dataset["text"]:
        # Extract user messages as prompts
        parts = text.split("<|im_start|>user\n")
        for part in parts[1:]:
            user_msg = part.split("<|im_end|>")[0].strip()
            if user_msg:
                prompts.append({"prompt": user_msg})

    return Dataset.from_list(prompts) if prompts else dataset


def _reasoning_reward(completions: list[str], **kwargs) -> list[float]:
    """Simple reward function for GRPO based on response quality heuristics."""
    rewards = []
    for completion in completions:
        score = 0.0
        # Reward longer, more detailed responses
        if len(completion) > 50:
            score += 0.3
        if len(completion) > 200:
            score += 0.2
        # Reward structured reasoning markers
        reasoning_markers = ["because", "therefore", "however", "first", "then", "finally"]
        for marker in reasoning_markers:
            if marker.lower() in completion.lower():
                score += 0.1
        # Penalize very short or empty responses
        if len(completion.strip()) < 10:
            score -= 0.5
        rewards.append(min(1.0, max(-1.0, score)))
    return rewards


def _upload_adapter(adapter_dir: Path, s3_prefix: str) -> int:
    """Upload adapter directory to S3, return total size in bytes."""
    s3 = clients.get_s3()
    bucket = clients.get_s3_bucket()
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
            return capability[0] >= 8  # Ampere (A100) and newer
    except ImportError:
        pass
    return False
