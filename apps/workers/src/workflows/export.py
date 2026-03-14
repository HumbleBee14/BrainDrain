"""Export workflow — merges LoRA, converts to GGUF, quantizes, uploads.

Triggered by POST /api/v1/models/{model_id}/exports.
"""

from temporalio import workflow

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.export_gguf import ExportGgufInput, ExportGgufOutput


@workflow.defn
class ExportWorkflow:
    """Export a fine-tuned model as a quantized GGUF file.

    Input: tenant_id, model_id, export_id, adapter details, quant_type.
    Produces a GGUF file uploaded to S3.
    """

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        model_id: str,
        export_id: str,
        adapter_path: str,
        base_model: str,
        quant_type: str,
    ) -> ExportGgufOutput:
        result = await workflow.execute_activity(
            "export_gguf",
            ExportGgufInput(
                tenant_id=tenant_id,
                model_id=model_id,
                export_id=export_id,
                adapter_path=adapter_path,
                base_model=base_model,
                quant_type=quant_type,
            ),
            start_to_close_timeout=timeouts.export_activity(),
            heartbeat_timeout=timeouts.export_heartbeat(),
            retry_policy=workflow.RetryPolicy(maximum_attempts=2),
        )

        return result
