"""Refine workflow — generates synthetic training pairs from parsed documents.

Triggered after documents are parsed, or manually by the user.
"""

from datetime import timedelta

from temporalio import workflow

with workflow.unsafe.imports_passed_through():
    from src.activities.stubs import (
        GenerateSyntheticPairsInput,
        GenerateSyntheticPairsOutput,
        generate_synthetic_pairs,
    )


@workflow.defn
class RefineWorkflow:
    """Generate instruction/response pairs from parsed documents.

    Input: tenant_id, project_id, document IDs, task type, config.
    Calls the synthetic pair generation activity.
    """

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        project_id: str,
        document_ids: list[str],
        task_type: str,
        config: dict,
    ) -> GenerateSyntheticPairsOutput:
        result = await workflow.execute_activity(
            generate_synthetic_pairs,
            GenerateSyntheticPairsInput(
                tenant_id=tenant_id,
                project_id=project_id,
                document_ids=document_ids,
                task_type=task_type,
                config=config,
            ),
            start_to_close_timeout=timedelta(minutes=30),
            retry_policy=workflow.RetryPolicy(maximum_attempts=2),
        )

        return result
