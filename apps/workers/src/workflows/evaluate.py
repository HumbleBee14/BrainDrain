"""Evaluate workflow — runs model evaluation after training.

Triggered automatically after training completes, or manually.
"""

from datetime import timedelta

from temporalio import workflow

with workflow.unsafe.imports_passed_through():
    from src.activities.stubs import (
        RunEvaluationInput,
        RunEvaluationOutput,
        run_evaluation,
    )


@workflow.defn
class EvaluateWorkflow:
    """Evaluate a fine-tuned model against held-out data.

    Input: tenant_id, model_id, evaluation_id, adapter details.
    Produces scores and a human-readable report.
    """

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        model_id: str,
        evaluation_id: str,
        adapter_path: str,
        base_model: str,
        dataset_path: str,
    ) -> RunEvaluationOutput:
        result = await workflow.execute_activity(
            run_evaluation,
            RunEvaluationInput(
                tenant_id=tenant_id,
                model_id=model_id,
                evaluation_id=evaluation_id,
                adapter_path=adapter_path,
                base_model=base_model,
                dataset_path=dataset_path,
            ),
            start_to_close_timeout=timedelta(hours=1),
            retry_policy=workflow.RetryPolicy(maximum_attempts=2),
        )

        return result
