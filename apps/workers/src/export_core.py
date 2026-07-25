"""GGUF export compute: merge LoRA, convert, quantize, upload.

Pure compute + object storage, no Temporal or DB access, so the same code runs
in-process (local provider) or inside a Modal container (cloud provider).
"""

import logging
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

logger = logging.getLogger("platform.export")

_CONVERT_SCRIPT_CANDIDATES = (
    "/opt/llama.cpp/convert_hf_to_gguf.py",
    str(Path.home() / "llama.cpp" / "convert_hf_to_gguf.py"),
)
_QUANTIZE_BIN_CANDIDATES = (
    "/opt/llama.cpp/build/bin/llama-quantize",
    str(Path.home() / "llama.cpp" / "build" / "bin" / "llama-quantize"),
)


def _resolve_tool(name: str, candidates: tuple[str, ...]) -> str:
    found = shutil.which(name)
    if found:
        return found
    for candidate in candidates:
        if Path(candidate).exists():
            return candidate
    raise RuntimeError(
        f"{name} not found. Install llama.cpp and put {name} on PATH."
    )


def _s3_client():
    import boto3
    from botocore.config import Config

    endpoint = os.environ.get("APP_S3_ENDPOINT") or None
    return boto3.client(
        "s3",
        endpoint_url=endpoint,
        region_name=os.environ.get("APP_S3_REGION", "auto"),
        aws_access_key_id=os.environ.get("APP_S3_ACCESS_KEY"),
        aws_secret_access_key=os.environ.get("APP_S3_SECRET_KEY"),
        config=Config(signature_version="s3v4"),
    )


def _download_prefix(s3, bucket: str, prefix: str, local_dir: str) -> None:
    paginator = s3.get_paginator("list_objects_v2")
    downloaded = 0
    for page in paginator.paginate(Bucket=bucket, Prefix=prefix):
        for obj in page.get("Contents", []):
            key = obj["Key"]
            rel = key[len(prefix) :].lstrip("/") or os.path.basename(key)
            dest = os.path.join(local_dir, rel)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            s3.download_file(bucket, key, dest)
            downloaded += 1
    if downloaded == 0:
        raise RuntimeError(f"No adapter objects under s3://{bucket}/{prefix}")


def _merge_lora(adapter_dir: str, base_model: str, output_dir: str) -> None:
    import torch
    from peft import PeftModel
    from transformers import AutoModelForCausalLM, AutoTokenizer

    tokenizer = AutoTokenizer.from_pretrained(base_model)
    # CPU on purpose: the merge is memory-bound and device_map="auto" fails on
    # non-CUDA hosts. fp16 is only the intermediate; quant_type sets final precision.
    model = AutoModelForCausalLM.from_pretrained(
        base_model, torch_dtype=torch.float16, device_map={"": "cpu"}
    )
    model = PeftModel.from_pretrained(model, adapter_dir)
    model = model.merge_and_unload()

    os.makedirs(output_dir, exist_ok=True)
    model.save_pretrained(output_dir)
    tokenizer.save_pretrained(output_dir)


def _run_tool(cmd: list[str], failure: str) -> None:
    result = subprocess.run(
        cmd, capture_output=True, text=True, timeout=3600, check=False
    )
    if result.returncode != 0:
        raise RuntimeError(f"{failure}: {result.stderr[:500]}")


def run_export_core(payload: dict) -> dict:
    """Produce a quantized GGUF and upload it.

    payload: tenant_id, model_id, export_id, adapter_path, base_model, quant_type
    returns: {"storage_path": str, "file_size_bytes": int}
    """
    bucket = os.environ["APP_S3_BUCKET"]
    s3 = _s3_client()
    quant_type = payload["quant_type"]

    work_dir = tempfile.mkdtemp(prefix="gguf-export-")
    try:
        adapter_dir = os.path.join(work_dir, "adapter")
        os.makedirs(adapter_dir, exist_ok=True)
        _download_prefix(s3, bucket, payload["adapter_path"], adapter_dir)

        merged_dir = os.path.join(work_dir, "merged")
        _merge_lora(adapter_dir, payload["base_model"], merged_dir)

        f16_gguf = os.path.join(work_dir, "model-f16.gguf")
        _run_tool(
            [
                "python",
                _resolve_tool("convert_hf_to_gguf.py", _CONVERT_SCRIPT_CANDIDATES),
                merged_dir,
                "--outfile",
                f16_gguf,
                "--outtype",
                "f16",
            ],
            "GGUF conversion failed",
        )

        final_gguf = os.path.join(work_dir, f"model-{quant_type}.gguf")
        _run_tool(
            [
                _resolve_tool("llama-quantize", _QUANTIZE_BIN_CANDIDATES),
                f16_gguf,
                final_gguf,
                quant_type,
            ],
            "Quantization failed",
        )

        file_size = os.path.getsize(final_gguf)
        s3_key = (
            f"exports/{payload['tenant_id']}/{payload['model_id']}/"
            f"{payload['export_id']}/{quant_type}.gguf"
        )
        s3.upload_file(final_gguf, bucket, s3_key)

        logger.info("Export uploaded: %s (%d bytes)", s3_key, file_size)
        return {"storage_path": s3_key, "file_size_bytes": file_size}
    finally:
        shutil.rmtree(work_dir, ignore_errors=True)
