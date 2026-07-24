"""GGUF export activity — merge LoRA, convert to GGUF, quantize, upload.

Steps:
1. Download LoRA adapter from S3
2. Download/cache base model from HuggingFace
3. Merge LoRA into base model (peft)
4. Convert merged model to F16 GGUF (llama.cpp)
5. Quantize GGUF to requested type (Q4_K_M, Q5_K_M, etc.)
6. Upload final GGUF to S3
7. Update export record in DB with status + file size + path
"""

import asyncio
import logging
import os
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

from temporalio import activity

from src.infra import InfraContainer

logger = logging.getLogger("platform.export")


@dataclass
class ExportGgufInput:
    tenant_id: str
    model_id: str
    export_id: str
    adapter_path: str
    base_model: str
    quant_type: str


@dataclass
class ExportGgufOutput:
    storage_path: str
    file_size_bytes: int


class ExportGgufActivity:
    """Merge LoRA adapter into base model and export as quantized GGUF."""

    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="export_gguf")
    async def run(self, input: ExportGgufInput) -> ExportGgufOutput:
        db = self.infra.db
        s3 = self.infra.s3
        bucket = self.infra.s3_bucket

        work_dir = tempfile.mkdtemp(prefix="gguf-export-")
        try:
            # Update status to processing
            await db.execute(
                "UPDATE model_exports SET status = 'processing' WHERE id = $1 AND tenant_id = $2",
                input.export_id,
                input.tenant_id,
            )

            activity.heartbeat("Downloading adapter from S3...")
            adapter_dir = os.path.join(work_dir, "adapter")
            os.makedirs(adapter_dir, exist_ok=True)
            await self._download_adapter(s3, bucket, input.adapter_path, adapter_dir)

            activity.heartbeat("Merging LoRA into base model...")
            merged_dir = os.path.join(work_dir, "merged")
            await self._merge_lora(adapter_dir, input.base_model, merged_dir)

            activity.heartbeat("Converting to GGUF (F16)...")
            f16_gguf = os.path.join(work_dir, "model-f16.gguf")
            await self._convert_to_gguf(merged_dir, f16_gguf)

            activity.heartbeat(f"Quantizing to {input.quant_type}...")
            final_gguf = os.path.join(work_dir, f"model-{input.quant_type}.gguf")
            await self._quantize(f16_gguf, final_gguf, input.quant_type)

            file_size = os.path.getsize(final_gguf)

            activity.heartbeat("Uploading GGUF to S3...")
            s3_key = (
                f"exports/{input.tenant_id}/{input.model_id}/"
                f"{input.export_id}/{input.quant_type}.gguf"
            )
            await asyncio.to_thread(s3.upload_file, final_gguf, bucket, s3_key)

            # Update DB with success
            await db.execute(
                """UPDATE model_exports
                   SET status = 'completed',
                       storage_path = $3,
                       file_size_bytes = $4,
                       completed_at = now()
                   WHERE id = $1 AND tenant_id = $2""",
                input.export_id,
                input.tenant_id,
                s3_key,
                file_size,
            )

            logger.info(
                "Export completed: %s (%s, %d bytes)",
                input.export_id,
                input.quant_type,
                file_size,
            )

            return ExportGgufOutput(
                storage_path=s3_key,
                file_size_bytes=file_size,
            )

        except Exception as exc:
            # Update DB with failure
            error_msg = str(exc)[:500]
            await db.execute(
                """UPDATE model_exports
                   SET status = 'failed',
                       error = $3,
                       completed_at = now()
                   WHERE id = $1 AND tenant_id = $2""",
                input.export_id,
                input.tenant_id,
                error_msg,
            )
            raise

        finally:
            shutil.rmtree(work_dir, ignore_errors=True)

    async def _download_adapter(self, s3, bucket: str, adapter_path: str, local_dir: str) -> None:
        """Download all adapter files from S3 prefix."""

        # boto3 paginators are synchronous — collect the pages in a thread
        # rather than blocking the event loop (or `async for`-ing a sync
        # iterator, which raises TypeError).
        def _list_pages():
            paginator = s3.get_paginator("list_objects_v2")
            return list(paginator.paginate(Bucket=bucket, Prefix=adapter_path))

        pages = await asyncio.to_thread(_list_pages)

        for page in pages:
            for obj in page.get("Contents", []):
                key = obj["Key"]
                rel = key[len(adapter_path) :].lstrip("/")
                if not rel:
                    # Single file, not a directory prefix
                    rel = os.path.basename(key)
                local_path = os.path.join(local_dir, rel)
                os.makedirs(os.path.dirname(local_path), exist_ok=True)
                await asyncio.to_thread(s3.download_file, bucket, key, local_path)
                activity.heartbeat(f"Downloaded {rel}")

    async def _merge_lora(self, adapter_dir: str, base_model: str, output_dir: str) -> None:
        """Merge LoRA adapter into base model using peft.

        Runs in a thread to avoid blocking the async event loop.
        """

        def _do_merge():
            # Deferred: [ml] extra deps — top-level would crash non-ML workers at boot.
            import torch
            from peft import PeftModel
            from transformers import AutoModelForCausalLM, AutoTokenizer

            tokenizer = AutoTokenizer.from_pretrained(base_model)
            # CPU on purpose: merge is memory-bound; device_map="auto" crashes on
            # non-CUDA hosts. fp16 is only the intermediate; quant_type decides precision.
            model = AutoModelForCausalLM.from_pretrained(
                base_model,
                torch_dtype=torch.float16,
                device_map={"": "cpu"},
            )

            model = PeftModel.from_pretrained(model, adapter_dir)
            model = model.merge_and_unload()

            os.makedirs(output_dir, exist_ok=True)
            model.save_pretrained(output_dir)
            tokenizer.save_pretrained(output_dir)

            logger.info("LoRA merged into base model at %s", output_dir)

        await asyncio.to_thread(_do_merge)

    async def _convert_to_gguf(self, model_dir: str, output_path: str) -> None:
        """Convert HF model to F16 GGUF using llama.cpp's convert script."""
        convert_script = shutil.which("convert_hf_to_gguf.py") or str(
            Path.home() / "llama.cpp" / "convert_hf_to_gguf.py"
        )

        if not Path(convert_script).exists():
            raise RuntimeError(
                f"convert_hf_to_gguf.py not found at '{convert_script}'. "
                "Please install llama.cpp and ensure convert_hf_to_gguf.py is on PATH."
            )

        result = await asyncio.to_thread(
            subprocess.run,
            ["python", convert_script, model_dir, "--outfile", output_path, "--outtype", "f16"],
            capture_output=True,
            text=True,
            timeout=3600,
            check=False,
        )

        if result.returncode != 0:
            raise RuntimeError(f"GGUF conversion failed: {result.stderr[:500]}")

        logger.info("Converted to F16 GGUF: %s", output_path)

    async def _quantize(self, input_path: str, output_path: str, quant_type: str) -> None:
        """Quantize F16 GGUF to target quantization type using llama-quantize."""
        quantize_bin = shutil.which("llama-quantize") or str(
            Path.home() / "llama.cpp" / "build" / "bin" / "llama-quantize"
        )

        if not Path(quantize_bin).exists():
            raise RuntimeError(
                f"llama-quantize not found at '{quantize_bin}'. "
                "Please install llama.cpp and ensure llama-quantize is on PATH."
            )

        result = await asyncio.to_thread(
            subprocess.run,
            [quantize_bin, input_path, output_path, quant_type],
            capture_output=True,
            text=True,
            timeout=3600,
            check=False,
        )

        if result.returncode != 0:
            raise RuntimeError(f"Quantization failed: {result.stderr[:500]}")

        logger.info("Quantized to %s: %s", quant_type, output_path)
