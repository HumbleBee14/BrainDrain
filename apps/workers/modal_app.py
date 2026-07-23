"""Modal deployment for cloud GPU fine-tuning.

Deploy once (not per job):  modal deploy apps/workers/modal_app.py
The worker then invokes it via modal.Function.from_name(...).spawn.aio(...).

The remote `train` function runs the SAME pure-compute core as the local
provider (src.activities.train_model.run_training_core). It touches only S3 +
the judge LLM — never Postgres, and never Redis (metrics are forced to the
log-only sink below; those stay on the worker side).

GPU type is chosen per-call by the worker via `.with_options(gpu=...)`, so a
single deployed function serves every gpu_class.
"""

import modal

# Base deps (temporalio/asyncpg/redis/boto3/httpx/pydantic*, needed transitively
# at import time — see comment below) plus the pyproject [ml] extra. This is NOT
# a literal mirror of either dependency group — see docs/CLOUD_GPU_TRAINING.md
# §8. Modal builds this image on its own infra.
image = (
    modal.Image.debian_slim(python_version="3.11")
    .pip_install(
        "unsloth>=2025.12",
        "transformers>=4.51.0,<5.0.0",
        "datasets>=3.2.0",
        "trl>=0.16.0",
        "peft>=0.14.0",
        "accelerate>=1.2.0",
        "bitsandbytes>=0.45.0",
        "pynvml>=12.0.0",
        "boto3>=1.36.0",
        "httpx>=0.27.0",
        "pydantic>=2.10.0",
        "pydantic-settings>=2.7.0",
        # Transitively imported at load time by src.activities.stubs /
        # src.activities.train_model (via src.infra): temporalio (`from
        # temporalio import activity`), asyncpg, redis. Not used at runtime
        # remotely (remote never touches Postgres/Redis — see I3/§2 in
        # docs/CLOUD_GPU_TRAINING.md) but required for the module graph to
        # import without ModuleNotFoundError. Versions match apps/workers/pyproject.toml.
        "temporalio>=1.9.0",
        "asyncpg>=0.29.0",
        "redis>=5.0.0",
    )
    # Make our own `src` package importable remotely (replaces removed auto-mount).
    # Must be the last layer since copy defaults to False (mounted, not built).
    .add_local_python_source("src")
)

app = modal.App("platform-training")

# Secret must provide: APP_S3_* , APP_LLM_* , APP_HF_TOKEN, and a syntactically
# valid APP_DATABASE_URL placeholder (never connected to). See docs/CLOUD_GPU_TRAINING.md.
_secret = modal.Secret.from_name("platform-training-secrets")


def _remote_env_setup():
    """Container-start env defaults shared by every remote function.

    Metrics sink: a compose-internal Redis (redis://localhost:6379) is
    unreachable from Modal, so DEFAULT to the log-only sink — but let the
    secret override it: set APP_METRICS_BACKEND=redis AND a PUBLIC
    APP_REDIS_URL (e.g. an Upstash rediss:// URL Modal can reach) in the
    Modal secret to stream live per-step metrics instead. Using setdefault
    (not hard assignment) preserves that override while keeping the safe
    default when no reachable Redis is configured. Must run BEFORE
    build_settings() reads WorkerSettings from the environment. Do not change
    the local default in config.py.
    """
    import os

    os.environ.setdefault("HF_HOME", "/tmp/hf_cache")
    os.environ.setdefault("APP_METRICS_BACKEND", "log")


@app.function(image=image, gpu="A10", timeout=86400, secrets=[_secret])
async def train(payload: dict) -> dict:
    """Remote GPU entrypoint. payload = {"input": {...}, "llm_config": {...}}."""
    _remote_env_setup()

    from src.activities.stubs import StartTrainingInput
    from src.activities.train_model import run_training_core
    from src.modal_runtime import build_s3_client, build_settings
    from src.tenant_config import TenantLlmConfig

    settings = build_settings()
    s3, bucket = build_s3_client(settings)

    result = await run_training_core(
        StartTrainingInput(**payload["input"]),
        s3=s3,
        s3_bucket=bucket,
        settings=settings,
        llm_config=TenantLlmConfig(**payload["llm_config"]),
    )
    return {
        "adapter_path": result.adapter_path,
        "adapter_size_bytes": result.adapter_size_bytes,
        "metrics": result.metrics,
    }


@app.function(image=image, gpu="A10", timeout=86400, secrets=[_secret])
async def train_sft_round(payload: dict) -> dict:
    """Remote SFT round for the iterative workflow. payload = {"input": {...}}."""
    _remote_env_setup()

    from src.activities.stubs import TrainSftRoundInput
    from src.activities.train_model import run_sft_round_core
    from src.modal_runtime import build_s3_client, build_settings

    settings = build_settings()
    s3, bucket = build_s3_client(settings)

    result = await run_sft_round_core(
        TrainSftRoundInput(**payload["input"]),
        s3=s3,
        s3_bucket=bucket,
        settings=settings,
    )
    return {
        "adapter_path": result.adapter_path,
        "adapter_size_bytes": result.adapter_size_bytes,
        "metrics": result.metrics,
    }


@app.function(image=image, gpu="A10", timeout=86400, secrets=[_secret])
async def evaluate_holdout(payload: dict) -> dict:
    """Remote holdout eval for the iterative workflow. payload = {"input": {...}}."""
    _remote_env_setup()

    from src.activities.stubs import EvaluateHoldoutInput
    from src.activities.train_model import run_evaluate_holdout_core
    from src.modal_runtime import build_s3_client, build_settings

    settings = build_settings()
    s3, bucket = build_s3_client(settings)

    result = await run_evaluate_holdout_core(
        EvaluateHoldoutInput(**payload["input"]),
        s3=s3,
        s3_bucket=bucket,
        settings=settings,
    )
    return {"eval_loss": result.eval_loss, "metrics": result.metrics}


@app.function(image=image, gpu="A10", timeout=86400, secrets=[_secret])
async def run_evaluation(payload: dict) -> dict:
    """Remote full evaluation suite. payload = {"input": {...}, "llm_config": {...}}."""
    _remote_env_setup()

    from src.activities.run_evaluation import run_evaluation_core
    from src.activities.stubs import RunEvaluationInput
    from src.modal_runtime import build_s3_client, build_settings
    from src.tenant_config import TenantLlmConfig

    settings = build_settings()
    s3, bucket = build_s3_client(settings)

    result = await run_evaluation_core(
        RunEvaluationInput(**payload["input"]),
        s3=s3,
        s3_bucket=bucket,
        settings=settings,
        llm_config=TenantLlmConfig(**payload["llm_config"]),
    )
    return {"scores": result.scores, "report": result.report}
