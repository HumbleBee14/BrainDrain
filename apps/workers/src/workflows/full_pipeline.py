"""Full pipeline workflow — orchestrates the entire flow from upload to deployment.

This is the "one-click fine-tune" workflow that chains:
Ingest → Refine (includes dataset build) → Train → Evaluate → Deploy
"""

from datetime import timedelta

from temporalio import workflow

with workflow.unsafe.imports_passed_through():
    from src.activities.stubs import DeployModelInput
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
        # Downstream activities persist these ids as UUID primary keys (training
        # jobs, models, evaluations) and route the deploy call by model id, so
        # they must be real UUIDs — not fabricated "<project>-job" strings.
        # workflow.uuid4() is the Temporal-deterministic UUID generator.
        training_job_id = str(workflow.uuid4())
        model_id = str(workflow.uuid4())
        evaluation_id = str(workflow.uuid4())

        # Stage 1: Ingest — parse uploaded documents
        ingest_result = await workflow.execute_child_workflow(
            IngestWorkflow.run,
            args=[tenant_id, project_id, document_ids],
            id=f"ingest-{project_id}",
        )

        # Stage 2: Refine — chunk, generate pairs, build dataset
        # RefineWorkflow internally chains: chunk_text → generate_pairs → build_dataset
        refine_result = await workflow.execute_child_workflow(
            RefineWorkflow.run,
            args=[tenant_id, project_id, document_ids, task_type, training_config],
            id=f"refine-{project_id}",
        )

        # Stage 3: Train
        train_result = await workflow.execute_child_workflow(
            TrainWorkflow.run,
            args=[
                tenant_id,
                training_job_id,
                refine_result.storage_path,
                base_model,
                training_config.get("method", "qlora"),
                # TrainWorkflow accepts modes quick/iterative/aligned/reasoning.
                # "sft" is not a valid mode; default to "quick".
                training_config.get("mode", "quick"),
                training_config.get("hyperparams", {}),
                training_config.get("gpu_class"),
            ],
            id=f"train-{project_id}",
        )

        # Stage 4: Evaluate
        eval_result = await workflow.execute_child_workflow(
            EvaluateWorkflow.run,
            args=[
                tenant_id,
                model_id,
                evaluation_id,
                train_result.adapter_path,
                base_model,
                refine_result.storage_path,
                "",  # judge_model — use tenant/default config
                "",  # judge_api_base — use tenant/default config
                training_config.get("gpu_class"),  # eval on the same GPU class as training
            ],
            id=f"evaluate-{project_id}",
        )

        # Stage 5: Deploy (optional, based on config)
        deploy_result = None
        if training_config.get("auto_deploy", False):
            deploy_result = await workflow.execute_activity(
                "deploy_model",
                DeployModelInput(
                    tenant_id=tenant_id,
                    model_id=model_id,
                    adapter_path=train_result.adapter_path,
                    base_model=base_model,
                    deployment_config=training_config.get("deployment", {}),
                ),
                # DeployModelActivity is registered on the GPU worker
                # (ml-pipeline-gpu). Without this the activity is scheduled on the
                # workflow's own queue, where it is not registered, and never runs.
                task_queue="ml-pipeline-gpu",
                start_to_close_timeout=timedelta(minutes=10),
            )

        return {
            "project_id": project_id,
            "documents_processed": ingest_result["documents_processed"],
            "dataset_pairs": refine_result.pair_count,
            "training_metrics": train_result.metrics,
            "eval_scores": eval_result.scores,
            "deployed": deploy_result is not None,
        }
