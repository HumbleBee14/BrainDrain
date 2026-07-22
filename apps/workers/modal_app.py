"""Modal deployment for cloud GPU fine-tuning.

Deploy once (not per job):  modal deploy apps/workers/modal_app.py
The worker then invokes it via modal.Function.from_name(...).spawn.aio(...).

The remote `train` function runs the SAME pure-compute core as the local
provider (src.activities.train_model.run_training_core). It touches only S3 +
the judge LLM — never Postgres/Redis (those stay on the worker side).

GPU type is chosen per-call by the worker via `.options(gpu=...)`, so a single
deployed function serves every gpu_class.
"""

import modal

# Mirror the pyproject [ml] extra. Modal builds this image on its own infra.
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
        "boto3>=1.35.0",
        "httpx>=0.27.0",
        "pydantic>=2.10.0",
        "pydantic-settings>=2.7.0",
    )
    # Make our own `src` package importable remotely (replaces removed auto-mount).
    # Must be the last layer since copy defaults to False (mounted, not built).
    .add_local_python_source("src")
)

app = modal.App("platform-training")

# Secret must provide: APP_S3_* , APP_LLM_* , APP_HF_TOKEN, and a syntactically
# valid APP_DATABASE_URL placeholder (never connected to). See docs/CLOUD_GPU_TRAINING.md.
_secret = modal.Secret.from_name("platform-training-secrets")


@app.function(image=image, gpu="A10", timeout=86400, secrets=[_secret])
async def train(payload: dict) -> dict:
    """Remote GPU entrypoint. payload = {"input": {...}, "llm_config": {...}}."""
    import os

    os.environ.setdefault("HF_HOME", "/tmp/hf_cache")

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
