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

import logging
from dataclasses import dataclass

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

    def __init__(self, infra: InfraContainer, gpu_provider=None):
        self.infra = infra
        self.gpu_provider = gpu_provider

    @activity.defn(name="export_gguf")
    async def run(self, input: ExportGgufInput) -> ExportGgufOutput:
        db = self.infra.db

        try:
            await db.execute(
                "UPDATE model_exports SET status = 'processing' WHERE id = $1 AND tenant_id = $2",
                input.export_id,
                input.tenant_id,
            )

            activity.heartbeat("Merging adapter and quantizing...")
            result = await self.gpu_provider.run_export_gguf(
                tenant_id=input.tenant_id,
                model_id=input.model_id,
                export_id=input.export_id,
                adapter_path=input.adapter_path,
                base_model=input.base_model,
                quant_type=input.quant_type,
            )
            s3_key = result["storage_path"]
            file_size = result["file_size_bytes"]

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

