"""Full pipeline workflow — orchestrates the entire flow from upload to deployment.

This is the "one-click fine-tune" workflow that chains:
Ingest → Refine → Build Dataset → Train → Evaluate → Deploy
"""

from datetime import timedelta

from temporalio import workflow

with workflow.unsafe.imports_passed_through():
    from src.activities.stubs import (
        BuildDatasetInput,
        DeployModelInput,
        build_dataset,
        deploy_model,
    )
    from src.workflows.evaluate import EvaluateWorkflow
    from src.workflows.ingest import IngestWorkflow
    from src.workflows.refine import RefineWorkflow
    from src.workflows.train import TrainWorkflow


@workflow.defn
class FullPipelineWorkflow:
    """End-to-end pipeline: upload → fine-tuned model.

    Chains child workflows for each stage. Each stage is independently
    retryable and visible in the Temporal UI for observability.
    """

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        project_id: str,
        document_ids: list[str],
        task_type: str,
        base_model: str,
        training_config: dict,
    ) -> dict:
        # Stage 1: Ingest — parse uploaded documents
        ingest_result = await workflow.execute_child_workflow(
            IngestWorkflow.run,
            args=[tenant_id, project_id, document_ids],
            id=f"ingest-{project_id}",
        )

        # Stage 2: Refine — generate synthetic pairs
        refine_result = await workflow.execute_child_workflow(
            RefineWorkflow.run,
            args=[tenant_id, project_id, document_ids, task_type, training_config],
            id=f"refine-{project_id}",
        )

        # Stage 3: Build dataset
        dataset_result = await workflow.execute_activity(
            build_dataset,
            BuildDatasetInput(
                tenant_id=tenant_id,
                project_id=project_id,
                dataset_id=f"{project_id}-ds",
                format="chat",
                config=training_config,
            ),
            start_to_close_timeout=timedelta(minutes=15),
        )

        # Stage 4: Train
        train_result = await workflow.execute_child_workflow(
            TrainWorkflow.run,
            args=[
                tenant_id,
                f"{project_id}-job",
                dataset_result.storage_path,
                base_model,
                training_config.get("method", "qlora"),
                training_config.get("mode", "sft"),
                training_config.get("hyperparams", {}),
                training_config.get("gpu_class"),
            ],
            id=f"train-{project_id}",
        )

        # Stage 5: Evaluate
        eval_result = await workflow.execute_child_workflow(
            EvaluateWorkflow.run,
            args=[
                tenant_id,
                f"{project_id}-model",
                f"{project_id}-eval",
                train_result.adapter_path,
                base_model,
                dataset_result.storage_path,
            ],
            id=f"evaluate-{project_id}",
        )

        # Stage 6: Deploy (optional, based on config)
        deploy_result = None
        if training_config.get("auto_deploy", False):
            deploy_result = await workflow.execute_activity(
                deploy_model,
                DeployModelInput(
                    tenant_id=tenant_id,
                    model_id=f"{project_id}-model",
                    adapter_path=train_result.adapter_path,
                    base_model=base_model,
                    deployment_config=training_config.get("deployment", {}),
                ),
                start_to_close_timeout=timedelta(minutes=10),
            )

        return {
            "project_id": project_id,
            "documents_processed": ingest_result["documents_processed"],
            "pairs_generated": refine_result.pair_count,
            "dataset_pairs": dataset_result.pair_count,
            "training_metrics": train_result.metrics,
            "eval_scores": eval_result.scores,
            "deployed": deploy_result is not None,
        }
