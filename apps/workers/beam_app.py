"""Beam deployment for cloud GPU fine-tuning (gpu_provider="beam").

Each task queue wraps the same pure-compute core as its Modal counterpart in
modal_app.py: S3 + judge LLM only, never Postgres/Redis. The worker submits
via BeamGpuProvider (src/gpu_provider.py) using the invoke URLs Beam prints
on deploy; record those in APP_BEAM_QUEUE_URLS.

A queue's GPU is fixed at deploy time (Beam has no per-call GPU override), so
deploy one variant per gpu_class you intend to serve and key the URL map as
"<function>@<gpu_class>":

    BEAM_DEPLOY_GPU=A10G beam deploy apps/workers/beam_app.py:train

Secrets (create once with `beam secret create <NAME> <value>`): the APP_S3_*
set, APP_LLM_* defaults, and APP_HF_TOKEN — same values as the Modal secret
`platform-training-secrets`.
"""

import os

from beam import Image, Volume, task_queue

_HF_CACHE_PATH = "/root/.cache/huggingface"

# Same literals as modal_app.py, for the same reason given there: this module
# is imported inside the container, where pyproject.toml (and modal_app, which
# needs the modal package) are unavailable. Drift is a test failure:
# tests/test_dockerfiles.py::test_beam_pins_match_modal_app.
TRL_UNSLOTH_VERSION = "0.24.0"

_RUNTIME_DEPS = (
    "temporalio>=1.9.0",
    "pydantic>=2.10.0",
    "pydantic-settings>=2.7.0",
    "pymupdf>=1.24.0",
    "python-docx>=1.1.0",
    "beautifulsoup4>=4.12.0",
    "markdown>=3.6",
    "langdetect>=1.0.9",
    "asyncpg>=0.29.0",
    "boto3>=1.36.0",
    "redis>=5.0.0",
    "httpx>=0.27.0",
    "cryptography>=42.0.0",
    "numpy>=1.26.0",
    "python-json-logger>=3.2.0",
    "opentelemetry-sdk>=1.29.0",
    "opentelemetry-exporter-otlp-proto-grpc>=1.29.0",
    "opentelemetry-instrumentation-logging>=0.50b0",
)

_DEPLOY_GPU = os.environ.get("BEAM_DEPLOY_GPU", "A10G")
_DEPLOY_TIMEOUT = int(os.environ.get("BEAM_DEPLOY_TIMEOUT_SECS", "86400"))

_SECRET_NAMES = [
    "APP_S3_ENDPOINT",
    "APP_S3_ACCESS_KEY",
    "APP_S3_SECRET_KEY",
    "APP_S3_BUCKET",
    "APP_S3_REGION",
    "APP_DATABASE_URL",
    "APP_HF_TOKEN",
]

_image = Image(python_version="python3.11").add_python_packages(
    [
        "unsloth==2026.8.4",
        "transformers>=4.51.0,<5.0.0",
        "datasets>=3.2.0",
        f"trl=={TRL_UNSLOTH_VERSION}",
        "peft>=0.14.0",
        "accelerate>=1.2.0",
        "bitsandbytes>=0.45.0",
        "pynvml>=12.0.0",
        *_RUNTIME_DEPS,
    ]
)

_volumes = [Volume(name="model-cache", mount_path=_HF_CACHE_PATH)]

# retries=0: Temporal owns retries; a Beam-level retry would re-run a GPU job
# invisibly to the platform's billing and reservation flow.
_queue_config = dict(
    image=_image,
    gpu=_DEPLOY_GPU,
    cpu=4,
    memory="16Gi",
    timeout=_DEPLOY_TIMEOUT,
    retries=0,
    secrets=_SECRET_NAMES,
    volumes=_volumes,
    env={"HF_HOME": _HF_CACHE_PATH, "APP_METRICS_BACKEND": "log"},
)


# Beam surfaces no traceback for a failed task (result is null; logs are
# reachable only over a websocket the platform cannot depend on), so every
# handler ships its outcome home in an envelope BeamGpuProvider unwraps.
def _enveloped(fn, payload: dict) -> dict:
    import traceback

    try:
        return {"ok": True, "result": fn(payload)}
    except Exception:
        return {"ok": False, "error": traceback.format_exc()}


@task_queue(name="train", **_queue_config)
def train(payload: dict) -> dict:
    return _enveloped(_train, payload)


def _train(payload: dict) -> dict:
    import asyncio

    from src.activities.stubs import StartTrainingInput
    from src.activities.train_model import run_training_core
    from src.modal_runtime import build_s3_client, build_settings
    from src.tenant_config import TenantLlmConfig

    settings = build_settings()
    s3, bucket = build_s3_client(settings)

    result = asyncio.run(
        run_training_core(
            StartTrainingInput(**payload["input"]),
            s3=s3,
            s3_bucket=bucket,
            settings=settings,
            llm_config=TenantLlmConfig(**payload["llm_config"]),
        )
    )
    return {
        "adapter_path": result.adapter_path,
        "adapter_size_bytes": result.adapter_size_bytes,
        "metrics": result.metrics,
    }


@task_queue(name="train_sft_round", **_queue_config)
def train_sft_round(payload: dict) -> dict:
    return _enveloped(_train_sft_round, payload)


def _train_sft_round(payload: dict) -> dict:
    import asyncio

    from src.activities.stubs import TrainSftRoundInput
    from src.activities.train_model import run_sft_round_core
    from src.modal_runtime import build_s3_client, build_settings

    settings = build_settings()
    s3, bucket = build_s3_client(settings)

    result = asyncio.run(
        run_sft_round_core(
            TrainSftRoundInput(**payload["input"]),
            s3=s3,
            s3_bucket=bucket,
            settings=settings,
        )
    )
    return {
        "adapter_path": result.adapter_path,
        "adapter_size_bytes": result.adapter_size_bytes,
        "metrics": result.metrics,
    }


@task_queue(name="evaluate_holdout", **_queue_config)
def evaluate_holdout(payload: dict) -> dict:
    return _enveloped(_evaluate_holdout, payload)


def _evaluate_holdout(payload: dict) -> dict:
    import asyncio

    from src.activities.stubs import EvaluateHoldoutInput
    from src.activities.train_model import run_evaluate_holdout_core
    from src.modal_runtime import build_s3_client, build_settings

    settings = build_settings()
    s3, bucket = build_s3_client(settings)

    result = asyncio.run(
        run_evaluate_holdout_core(
            EvaluateHoldoutInput(**payload["input"]),
            s3=s3,
            s3_bucket=bucket,
            settings=settings,
        )
    )
    return result


@task_queue(name="run_evaluation", **_queue_config)
def run_evaluation(payload: dict) -> dict:
    return _enveloped(_run_evaluation, payload)


def _run_evaluation(payload: dict) -> dict:
    import asyncio

    from src.activities.run_evaluation import run_evaluation_core
    from src.activities.stubs import RunEvaluationInput
    from src.modal_runtime import build_s3_client, build_settings
    from src.tenant_config import TenantLlmConfig

    settings = build_settings()
    s3, bucket = build_s3_client(settings)

    result = asyncio.run(
        run_evaluation_core(
            RunEvaluationInput(**payload["input"]),
            s3=s3,
            s3_bucket=bucket,
            settings=settings,
            llm_config=TenantLlmConfig(**payload["llm_config"]),
        )
    )
    return result
