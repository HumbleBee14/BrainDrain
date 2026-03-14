"""Refine workflow — generates synthetic training data from parsed documents.

Chains: chunk_text → generate_synthetic_pairs → build_dataset.
Triggered after documents are parsed, or manually by the user.
"""

import uuid

from temporalio import workflow

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.build_dataset import BuildDatasetInput, BuildDatasetOutput
    from src.activities.chunk_text import ChunkTextInput
    from src.activities.generate_pairs import GenerateSyntheticPairsInput


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
            retry_policy=workflow.RetryPolicy(maximum_attempts=3),
        )

        if chunk_result.chunk_count == 0:
            workflow.logger.warning("No chunks generated — nothing to refine")
            return BuildDatasetOutput(pair_count=0, storage_path="")

        # Stage 2: Generate synthetic pairs from chunks
        pairs_result = await workflow.execute_activity(
            "generate_synthetic_pairs",
            GenerateSyntheticPairsInput(
                tenant_id=tenant_id,
                project_id=project_id,
                chunks_storage_path=chunk_result.chunks_storage_path,
                task_type=task_type,
                pairs_per_chunk=config.get("pairs_per_chunk", 5),
            ),
            start_to_close_timeout=timeouts.generate_pairs_activity(),
            retry_policy=workflow.RetryPolicy(maximum_attempts=2),
            heartbeat_timeout=timeouts.generate_pairs_heartbeat(),
        )

        if pairs_result.pair_count == 0:
            workflow.logger.warning("No pairs generated")
            return BuildDatasetOutput(pair_count=0, storage_path="")

        # Stage 3: Build the dataset (filter, format, split)
        dataset_id = str(uuid.uuid4())
        dataset_result = await workflow.execute_activity(
            "build_dataset",
            BuildDatasetInput(
                tenant_id=tenant_id,
                project_id=project_id,
                dataset_id=dataset_id,
                pairs_storage_path=pairs_result.storage_path,
                system_prompt=config.get("system_prompt", ""),
            ),
            start_to_close_timeout=timeouts.build_dataset_activity(),
            retry_policy=workflow.RetryPolicy(maximum_attempts=2),
        )

        return dataset_result
