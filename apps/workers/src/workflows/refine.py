"""Refine workflow — generates synthetic training data from parsed documents.

Chains: chunk_text → generate_synthetic_pairs → build_dataset.
Triggered after documents are parsed, or manually by the user.
"""

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ApplicationError

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.build_dataset import BuildDatasetInput, BuildDatasetOutput
    from src.activities.chunk_text import ChunkTextInput, ChunkTextOutput
    from src.activities.generate_pairs import (
        GenerateSyntheticPairsInput,
        GenerateSyntheticPairsOutput,
    )
    from src.activities.pipeline_records import MarkDatasetFailedInput
    from src.failure_message import root_cause_message


async def _mark_failed(tenant_id: str, dataset_id: str, error: str) -> None:
    """Best-effort write of the failure onto the reserved dataset row.

    Swallows its own failure (logged) so Temporal still records the original.
    """
    try:
        await workflow.execute_activity(
            "mark_dataset_failed",
            MarkDatasetFailedInput(tenant_id=tenant_id, dataset_id=dataset_id, error=error),
            start_to_close_timeout=timeouts.db_lookup(),
            retry_policy=RetryPolicy(maximum_attempts=3),
        )
    except Exception:
        workflow.logger.exception("Failed to persist failed status for dataset %s", dataset_id)


@workflow.defn
class RefineWorkflow:
    """Generate training data from parsed documents.

    Input: tenant_id, project_id, document IDs, task type, config.
    Pipeline: chunk → generate pairs → build dataset.
    """

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        project_id: str,
        document_ids: list[str],
        task_type: str,
        config: dict,
    ) -> BuildDatasetOutput:
        # Reserved by the API before this run started; falls back for callers
        # that start the workflow directly.
        dataset_id = config.get("dataset_id") or str(workflow.uuid4())
        try:
            return await self._refine(
                tenant_id, project_id, document_ids, task_type, config, dataset_id
            )
        except Exception as e:
            await _mark_failed(tenant_id, dataset_id, root_cause_message(e))
            raise

    async def _refine(
        self,
        tenant_id: str,
        project_id: str,
        document_ids: list[str],
        task_type: str,
        config: dict,
        dataset_id: str,
    ) -> BuildDatasetOutput:
        # Stage 1: Chunk the parsed documents
        chunk_result = await workflow.execute_activity(
            "chunk_text",
            ChunkTextInput(
                tenant_id=tenant_id,
                project_id=project_id,
                document_ids=document_ids,
                chunk_size=config.get("chunk_size", 1500),
                overlap=config.get("overlap", 200),
            ),
            start_to_close_timeout=timeouts.chunk_activity(),
            retry_policy=RetryPolicy(maximum_attempts=3),
            result_type=ChunkTextOutput,
        )

        if chunk_result.chunk_count == 0:
            raise ApplicationError(
                "No text chunks came out of the parsed documents", non_retryable=True
            )

        # Stage 2: Generate synthetic pairs from chunks
        pairs_result = await workflow.execute_activity(
            "generate_synthetic_pairs",
            GenerateSyntheticPairsInput(
                tenant_id=tenant_id,
                project_id=project_id,
                chunks_storage_path=chunk_result.chunks_storage_path,
                task_type=task_type,
                pairs_per_chunk=config.get("pairs_per_chunk", 5),
                guidance=config.get("guidance", ""),
                facets=config.get("facets"),
                golden_holdout_ratio=config.get("golden_holdout_ratio", 0.1),
                facet_subtopics=config.get("facet_subtopics", 3),
            ),
            start_to_close_timeout=timeouts.generate_pairs_activity(),
            retry_policy=RetryPolicy(maximum_attempts=2),
            heartbeat_timeout=timeouts.generate_pairs_heartbeat(),
            result_type=GenerateSyntheticPairsOutput,
        )

        if pairs_result.pair_count == 0:
            raise ApplicationError(
                "The generator produced no usable training pairs", non_retryable=True
            )

        # Stage 3: Build the dataset (filter, format, split)
        dataset_result = await workflow.execute_activity(
            "build_dataset",
            BuildDatasetInput(
                tenant_id=tenant_id,
                project_id=project_id,
                dataset_id=dataset_id,
                pairs_storage_path=pairs_result.storage_path,
                system_prompt=config.get("system_prompt", ""),
                golden_storage_path=pairs_result.golden_storage_path,
            ),
            start_to_close_timeout=timeouts.build_dataset_activity(),
            retry_policy=RetryPolicy(maximum_attempts=2),
            result_type=BuildDatasetOutput,
        )

        return dataset_result
