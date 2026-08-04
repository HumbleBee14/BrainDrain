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
# Shared with the serving app so base weights are pulled from HuggingFace once
# and then reused by every subsequent training job and serving cold start.
_HF_CACHE_PATH = "/root/.cache/huggingface"
_weights_cache = modal.Volume.from_name("ekcron-model-cache", create_if_missing=True)

# Public Redis URL + metrics backend, so per-step training metrics reach the
# dashboard instead of the log-only fallback.
_metrics_secret = modal.Secret.from_name("ekcron-metrics-secrets")

# Mirror of pyproject.toml runtime deps. Importing a worker activity pulls in the
# whole worker module graph, so a dep missing here becomes a ModuleNotFoundError
# at GPU-call time. Kept in sync by
# tests/test_dockerfiles.py::test_modal_image_covers_pyproject_runtime_deps.
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
    "python-json-logger>=3.2.0",
    "opentelemetry-sdk>=1.29.0",
    "opentelemetry-exporter-otlp-proto-grpc>=1.29.0",
    "opentelemetry-instrumentation-logging>=0.50b0",
)

_base_image = (
    modal.Image.debian_slim(python_version="3.11")
    # GPU/ML stack — remote-only, absent from pyproject's runtime deps.
    .pip_install(
        "unsloth>=2025.12",
        "transformers>=4.51.0,<5.0.0",
        "datasets>=3.2.0",
        "trl>=0.16.0",
        "peft>=0.14.0",
        "accelerate>=1.2.0",
        "bitsandbytes>=0.45.0",
        "pynvml>=12.0.0",
    )
    .pip_install(*_RUNTIME_DEPS)
    .env({"HF_HOME": _HF_CACHE_PATH})
)

# `src` is mounted, not built, so it must be the final layer of any derived image.
image = _base_image.add_local_python_source("src")


# Teacher logprob extraction runs vLLM, which cannot share the training image:
# unsloth pins its own torch build and resolving both together produces a broken
# pair. vLLM brings transformers and numpy with it, so only the worker's own
# runtime deps are added — and they go first so vLLM's pins win the resolution,
# since vLLM is by far the more fragile half.
# Pinned to the version the extraction contract was measured against
# (docs/distillation/STAGE2-SPIKE-FINDINGS.md); prompt logprobs are known to vary
# across versions, so the pin is part of the artifact's provenance.
VLLM_EXTRACTION_VERSION = "0.26.0"

extract_image = (
    modal.Image.debian_slim(python_version="3.11")
    .pip_install(*_RUNTIME_DEPS)
    .pip_install(f"vllm=={VLLM_EXTRACTION_VERSION}")
    .env(
        {
            "HF_HOME": _HF_CACHE_PATH,
            # FlashInfer's sampler JIT-compiles a CUDA kernel at engine warmup and
            # fails without nvcc, which a slim image has no reason to carry.
            # Scoring never samples, so switching it off is free. Measured on a
            # real GPU start, not a precaution.
            "VLLM_USE_FLASHINFER_SAMPLER": "0",
        }
    )
    .add_local_python_source("src")
)


# Export needs the same torch/peft stack plus llama.cpp's converter and quantizer.
export_image = (
    _base_image.apt_install("git", "cmake", "build-essential")
    .run_commands(
        "git clone --depth 1 https://github.com/ggml-org/llama.cpp /opt/llama.cpp",
        "pip install --no-cache-dir -r"
        " /opt/llama.cpp/requirements/requirements-convert_hf_to_gguf.txt",
        "cmake -S /opt/llama.cpp -B /opt/llama.cpp/build -DLLAMA_CURL=OFF -DBUILD_SHARED_LIBS=OFF",
        "cmake --build /opt/llama.cpp/build --target llama-quantize -j 4",
    )
    .add_local_python_source("src")
)

app = modal.App("platform-training")

# Secret must provide: APP_S3_* , APP_LLM_* , APP_HF_TOKEN, and a syntactically
# valid APP_DATABASE_URL placeholder (never connected to). See docs/CLOUD_GPU_TRAINING.md.
_secret = modal.Secret.from_name("platform-training-secrets")


def _remote_env_setup():
    """Container-start env defaults shared by every remote function.

    setdefault, so a secret providing a Modal-reachable Redis keeps live
    metrics; falls back to log-only otherwise. Must run before
    build_settings() reads the environment.
    """
    import os

    os.environ.setdefault("APP_METRICS_BACKEND", "log")


@app.function(
    image=image,
    gpu="A10",
    timeout=86400,
    secrets=[_secret, _metrics_secret],
    volumes={_HF_CACHE_PATH: _weights_cache},
)
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


@app.function(
    image=image,
    gpu="A10",
    timeout=86400,
    secrets=[_secret, _metrics_secret],
    volumes={_HF_CACHE_PATH: _weights_cache},
)
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


@app.function(
    image=image,
    gpu="A10",
    timeout=86400,
    secrets=[_secret, _metrics_secret],
    volumes={_HF_CACHE_PATH: _weights_cache},
)
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


@app.function(
    image=image,
    gpu="A10",
    timeout=86400,
    secrets=[_secret, _metrics_secret],
    volumes={_HF_CACHE_PATH: _weights_cache},
)
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


@app.function(
    image=extract_image,
    gpu="A10",
    timeout=86400,
    secrets=[_secret, _metrics_secret],
    volumes={_HF_CACHE_PATH: _weights_cache},
)
async def extract_logprobs(payload: dict) -> dict:
    """Remote teacher scoring pass. payload = {"input": {...}}."""
    _remote_env_setup()

    from src.activities.extract_logprobs import run_extract_logprobs_core
    from src.activities.stubs import ExtractTeacherLogprobsInput
    from src.modal_runtime import build_s3_client, build_settings

    settings = build_settings()
    s3, bucket = build_s3_client(settings)

    result = await run_extract_logprobs_core(
        ExtractTeacherLogprobsInput(**payload["input"]),
        s3=s3,
        s3_bucket=bucket,
        settings=settings,
    )
    return {
        "manifest_path": result.manifest_path,
        "artifact_prefix": result.artifact_prefix,
        "records": result.records,
        "scored_positions": result.scored_positions,
        "skipped_records": result.skipped_records,
        "shards": result.shards,
        "metrics": result.metrics,
    }


@app.function(
    image=export_image,
    cpu=4.0,
    memory=32768,
    timeout=7200,
    secrets=[_secret],
    volumes={_HF_CACHE_PATH: _weights_cache},
)
def export_gguf(payload: dict) -> dict:
    """Merge + quantize on CPU; no GPU is needed for either step."""
    _remote_env_setup()

    from src.export_core import run_export_core

    return run_export_core(payload)
