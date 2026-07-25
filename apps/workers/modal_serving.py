"""Modal scale-to-zero vLLM serving for tuned LoRA adapters.

Serves one base model with vLLM + the S3 LoRA resolver plugin (infra/serving/),
so any tenant adapter in R2 is loadable by name at request time. The container
scales to zero after `SCALEDOWN_WINDOW` seconds idle — no GPU cost while unused,
cold-start on the next request.

vLLM serves ONE base model per endpoint (many LoRAs on top), so deploy one app
per base model, overriding BASE_MODEL. The deployed web endpoint URL is
registered as an inference instance (backend_type=vllm); the control plane then
routes /v1/chat/completions to it, selecting the adapter via the `model` field.

Deploy:  modal deploy apps/workers/modal_serving.py
"""

import os
from pathlib import Path

import modal

VLLM_VERSION = "v0.8.5"
# Small/cheap default to prove the loop; override with BASE_MODEL in the secret
# (or redeploy a variant) to serve a different catalog base model.
DEFAULT_BASE_MODEL = "unsloth/Llama-3.2-1B-Instruct"

_resolver_src = Path(__file__).resolve().parents[2] / "infra/serving/vllm_s3_lora_resolver"

# Mirrors infra/serving/Dockerfile.vllm: the vLLM OpenAI image + our resolver
# plugin, with runtime LoRA updating enabled.
serving_image = (
    modal.Image.from_registry(f"vllm/vllm-openai:{VLLM_VERSION}")
    .entrypoint([])
    .run_commands("ln -sf $(which python3) /usr/local/bin/python")
    .env(
        {
            "VLLM_ALLOW_RUNTIME_LORA_UPDATING": "true",
            "VLLM_PLUGINS": "s3_lora_resolver",
            "VLLM_LORA_RESOLVER_CACHE_DIR": "/var/lora-cache",
        }
    )
    .add_local_dir(_resolver_src, remote_path="/opt/vllm_s3_lora_resolver", copy=True)
    .run_commands(
        "pip install --no-cache-dir /opt/vllm_s3_lora_resolver",
        "mkdir -p /var/lora-cache",
    )
)

app = modal.App("ekcron-vllm-serving")

# Reuses the training secret: provides APP_S3_* (bucket, endpoint, creds) that we
# remap below to the boto3/resolver-standard names. Optionally set BASE_MODEL,
# SERVING_GPU, SCALEDOWN_WINDOW, VLLM_DTYPE, MAX_LORA_RANK here to tune serving.
_secret = modal.Secret.from_name("platform-training-secrets")

# Serving-only credentials, kept separate so rotating VLLM_API_KEY never rewrites
# the training secret. Holds VLLM_API_KEY, which must match the control plane's
# INFERENCE_API_KEY.
_serving_secret = modal.Secret.from_name("ekcron-serving-secrets")

_GPU = os.environ.get("SERVING_GPU", "A10")
_SCALEDOWN = int(os.environ.get("SCALEDOWN_WINDOW", "300"))
# T4 (compute 7.5) has no bfloat16 support, so vLLM must run fp16 there.
_DEFAULT_DTYPE = "half" if _GPU.upper().startswith("T4") else "auto"


@app.function(
    image=serving_image,
    gpu=_GPU,
    secrets=[_secret, _serving_secret],
    scaledown_window=_SCALEDOWN,  # idle seconds before scale-to-zero
    timeout=3600,
    # min_containers defaults to 0 → genuine scale-to-zero.
)
@modal.web_server(port=8000, startup_timeout=900)
def serve():
    """Launch the vLLM OpenAI server; Modal proxies :8000 as the endpoint URL."""
    import subprocess

    base_model = os.environ.get("BASE_MODEL", DEFAULT_BASE_MODEL)
    # The resolver + boto3 read standard names; the platform secret uses APP_S3_*.
    os.environ.setdefault("S3_LORA_BUCKET", os.environ.get("APP_S3_BUCKET", ""))
    os.environ.setdefault("S3_ENDPOINT_URL", os.environ.get("APP_S3_ENDPOINT", ""))
    os.environ.setdefault("S3_REGION", os.environ.get("APP_S3_REGION", "auto"))
    os.environ.setdefault("AWS_ACCESS_KEY_ID", os.environ.get("APP_S3_ACCESS_KEY", ""))
    os.environ.setdefault("AWS_SECRET_ACCESS_KEY", os.environ.get("APP_S3_SECRET_KEY", ""))

    cmd = [
        "python",
        "-m",
        "vllm.entrypoints.openai.api_server",
        "--host",
        "0.0.0.0",
        "--port",
        "8000",
        "--model",
        base_model,
        "--enable-lora",
        "--max-lora-rank",
        os.environ.get("MAX_LORA_RANK", "64"),
        "--max-loras",
        os.environ.get("MAX_LORAS", "4"),
        "--dtype",
        os.environ.get("VLLM_DTYPE", _DEFAULT_DTYPE),
    ]

    api_key = os.environ.get("VLLM_API_KEY", "").strip()
    if api_key:
        cmd += ["--api-key", api_key]

    subprocess.Popen(cmd)
