"""Data Studio workflows — short interactive steps of the guided generation flow.

Each workflow is thin: run its generation activity, then run
`update_data_guide` to persist the result and terminal status. Positional
args mirror `RefineWorkflow.run` — the Rust orchestrator starts these with a
positional JSON array.

Every workflow here writes `status="failed"` on the data guide before
re-raising, so a failed run never leaves the guide stuck in a
`generating_*` state (the frontend polls the status indefinitely otherwise).
The failed-status write is best-effort: if it itself fails too, the original
exception still propagates so Temporal records the real failure.
"""

import uuid

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.build_dataset import BuildDatasetInput, BuildDatasetOutput
    from src.activities.chunk_text import ChunkTextInput
    from src.activities.datagen_activities import (
        GenerateFacetsInput,
        GeneratePreviewInput,
        RefineGuidanceInput,
        UpdateDataGuideInput,
    )
    from src.activities.generate_pairs import GenerateSyntheticPairsInput


async def _mark_failed(tenant_id: str, data_guide_id: str) -> None:
    """Best-effort write of `status="failed"` onto the data guide.

    Called from an `except` block right before re-raising. Swallows its own
    failure (logged) so the original exception is what Temporal records.
    """
    try:
        await workflow.execute_activity(
            "update_data_guide",
            UpdateDataGuideInput(
                tenant_id=tenant_id,
                data_guide_id=data_guide_id,
                status="failed",
            ),
            start_to_close_timeout=timeouts.db_lookup(),
            retry_policy=RetryPolicy(maximum_attempts=3),
        )
    except Exception:
        workflow.logger.exception(
            "Failed to persist failed status for data guide %s", data_guide_id
        )


@workflow.defn
class GenerateFacetsWorkflow:
    """Extract document-grounded facets for a project and persist them."""

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        project_id: str,
        data_guide_id: str,
        task_type: str,
        guidance: str = "",
        num_facets: int = 8,
        existing: list[str] | None = None,
    ) -> None:
        try:
            result = await workflow.execute_activity(
                "generate_facets",
                GenerateFacetsInput(
                    tenant_id=tenant_id,
                    project_id=project_id,
                    task_type=task_type,
                    guidance=guidance,
                    num_facets=num_facets,
                    existing=existing,
                ),
                start_to_close_timeout=timeouts.datagen_interactive_activity(),
                retry_policy=RetryPolicy(maximum_attempts=2),
            )
        except Exception:
            await _mark_failed(tenant_id, data_guide_id)
            raise

        await workflow.execute_activity(
            "update_data_guide",
            UpdateDataGuideInput(
                tenant_id=tenant_id,
                data_guide_id=data_guide_id,
                status="facets_ready",
                facets=result.facets,
            ),
            start_to_close_timeout=timeouts.db_lookup(),
            retry_policy=RetryPolicy(maximum_attempts=3),
        )


@workflow.defn
class GeneratePreviewWorkflow:
    """Generate a faithfulness-gated sample of pairs for the user to rate."""

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        project_id: str,
        data_guide_id: str,
        task_type: str,
        guidance: str = "",
        facets: list[dict] | None = None,
        num_samples: int = 6,
    ) -> None:
        try:
            result = await workflow.execute_activity(
                "generate_preview",
                GeneratePreviewInput(
                    tenant_id=tenant_id,
                    project_id=project_id,
                    task_type=task_type,
                    guidance=guidance,
                    facets=facets,
                    num_samples=num_samples,
                ),
                start_to_close_timeout=timeouts.datagen_interactive_activity(),
                retry_policy=RetryPolicy(maximum_attempts=2),
            )
        except Exception:
            await _mark_failed(tenant_id, data_guide_id)
            raise

        await workflow.execute_activity(
            "update_data_guide",
            UpdateDataGuideInput(
                tenant_id=tenant_id,
                data_guide_id=data_guide_id,
                status="ready",
                preview_samples=result.preview_samples,
            ),
            start_to_close_timeout=timeouts.db_lookup(),
            retry_policy=RetryPolicy(maximum_attempts=3),
        )


@workflow.defn
class RefineGuidanceWorkflow:
    """Run the metaprompter over rated samples and persist the improved guidance."""

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        project_id: str,
        data_guide_id: str,
        task_type: str,
        guidance: str,
        rated: list[dict],
    ) -> None:
        try:
            result = await workflow.execute_activity(
                "refine_guidance",
                RefineGuidanceInput(
                    tenant_id=tenant_id,
                    task_type=task_type,
                    guidance=guidance,
                    rated=rated,
                ),
                start_to_close_timeout=timeouts.datagen_interactive_activity(),
                retry_policy=RetryPolicy(maximum_attempts=2),
            )
        except Exception:
            await _mark_failed(tenant_id, data_guide_id)
            raise

        history_entry = {
            "ts": workflow.now().isoformat(),
            "prev_guidance": guidance,
            "new_guidance": result.guidance,
            "rationale": result.rationale,
        }

        await workflow.execute_activity(
            "update_data_guide",
            UpdateDataGuideInput(
                tenant_id=tenant_id,
                data_guide_id=data_guide_id,
                status="ready",
                guidance=result.guidance,
                refinement_history_entry=history_entry,
            ),
            start_to_close_timeout=timeouts.db_lookup(),
            retry_policy=RetryPolicy(maximum_attempts=3),
        )


@workflow.defn
class GenerateDatasetWorkflow:
    """Generate the full training dataset from the finalized guidance/facets.

    Chains the same three activities as `RefineWorkflow` (chunk_text →
    generate_synthetic_pairs → build_dataset), with the same timeouts and
    retry policies, then persists the resulting `dataset_id` and terminal
    status onto the data guide — the step `RefineWorkflow` (shared with the
    non-Data-Studio callers) doesn't perform.
    """

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        project_id: str,
        data_guide_id: str,
        task_type: str,
        guidance: str,
        facets: list[dict] | None,
        document_ids: list[str],
        system_prompt: str = "",
        rated: list[dict] | None = None,
    ) -> BuildDatasetOutput:
        try:
            chunk_result = await workflow.execute_activity(
                "chunk_text",
                ChunkTextInput(
                    tenant_id=tenant_id,
                    project_id=project_id,
                    document_ids=document_ids,
                ),
                start_to_close_timeout=timeouts.chunk_activity(),
                retry_policy=RetryPolicy(maximum_attempts=3),
            )

            dataset_id: str | None = None
            if chunk_result.chunk_count == 0:
                workflow.logger.warning("No chunks generated — nothing to generate a dataset from")
                dataset_result = BuildDatasetOutput(pair_count=0, storage_path="")
            else:
                pairs_result = await workflow.execute_activity(
                    "generate_synthetic_pairs",
                    GenerateSyntheticPairsInput(
                        tenant_id=tenant_id,
                        project_id=project_id,
                        chunks_storage_path=chunk_result.chunks_storage_path,
                        task_type=task_type,
                        guidance=guidance,
                        facets=facets,
                        rated=rated or [],
                    ),
                    start_to_close_timeout=timeouts.generate_pairs_activity(),
                    retry_policy=RetryPolicy(maximum_attempts=2),
                    heartbeat_timeout=timeouts.generate_pairs_heartbeat(),
                )

                if pairs_result.pair_count == 0:
                    workflow.logger.warning("No pairs generated")
                    dataset_result = BuildDatasetOutput(pair_count=0, storage_path="")
                else:
                    dataset_id = str(uuid.uuid4())
                    dataset_result = await workflow.execute_activity(
                        "build_dataset",
                        BuildDatasetInput(
                            tenant_id=tenant_id,
                            project_id=project_id,
                            dataset_id=dataset_id,
                            pairs_storage_path=pairs_result.storage_path,
                            system_prompt=system_prompt,
                            golden_storage_path=pairs_result.golden_storage_path,
                        ),
                        start_to_close_timeout=timeouts.build_dataset_activity(),
                        retry_policy=RetryPolicy(maximum_attempts=2),
                    )
        except Exception:
            await _mark_failed(tenant_id, data_guide_id)
            raise

        await workflow.execute_activity(
            "update_data_guide",
            UpdateDataGuideInput(
                tenant_id=tenant_id,
                data_guide_id=data_guide_id,
                status="completed",
                dataset_id=dataset_id,
            ),
            start_to_close_timeout=timeouts.db_lookup(),
            retry_policy=RetryPolicy(maximum_attempts=3),
        )

        return dataset_result
