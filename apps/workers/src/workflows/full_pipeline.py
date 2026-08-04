"""Full pipeline workflow — orchestrates the entire flow from upload to deployment.

This is the "one-click fine-tune" workflow that chains:
Ingest → Refine (includes dataset build) → Train → Evaluate → Deploy
"""

from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ApplicationError

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.pipeline_records import CreateEvaluationInput, CreateTrainingJobInput
    from src.activities.stubs import DeployModelInput, DeployModelOutput
    from src.workflows.evaluate import EvaluateWorkflow
    from src.workflows.ingest import IngestWorkflow
    from src.workflows.refine import RefineWorkflow
    from src.workflows.train import TrainWorkflow


@workflow.defn
class FullPipelineWorkflow:
    """End-to-end pipeline: upload → fine-tuned model.

    Chains child workflows for each stage. Each stage is independently
    retryable and visible in the Temporal UI for observability.

    The per-stage API routes create their training_jobs / evaluations rows
    before starting each workflow; here those rows are created mid-pipeline
    by activities, once their prerequisites (dataset, model) actually exist —
    training claims its row by id, evaluation updates its row by id, and
    deploy routes by the models row id the trainer created.
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

        # Stage 2: Refine — chunk, generate pairs, build dataset
        # RefineWorkflow internally chains: chunk_text → generate_pairs → build_dataset
        refine_result = await workflow.execute_child_workflow(
            RefineWorkflow.run,
            args=[tenant_id, project_id, document_ids, task_type, training_config],
            id=f"refine-{project_id}",
        )
        if refine_result.pair_count == 0 or not refine_result.dataset_id:
            raise ApplicationError(
                "Pipeline produced no training data (0 pairs) — check the source "
                "documents and generation settings",
                non_retryable=True,
            )

        # Stage 3: Train — create the job row first so training can claim it
        training_job_id = await workflow.execute_activity(
            "create_training_job",
            CreateTrainingJobInput(
                tenant_id=tenant_id,
                project_id=project_id,
                dataset_id=refine_result.dataset_id,
                base_model=base_model,
                method=training_config.get("method", "qlora"),
                # TrainWorkflow accepts modes quick/distill/iterative/aligned/
                # reasoning. "sft" is not a valid mode; default to "quick".
                mode=training_config.get("mode", "quick"),
                hyperparams=training_config.get("hyperparams", {}),
                gpu_class=training_config.get("gpu_class"),
                teacher_config=training_config.get("teacher"),
            ),
            start_to_close_timeout=timeouts.db_lookup(),
            retry_policy=RetryPolicy(maximum_attempts=3),
        )

        train_result = await workflow.execute_child_workflow(
            TrainWorkflow.run,
            args=[
                tenant_id,
                training_job_id,
                refine_result.storage_path,
                base_model,
                training_config.get("method", "qlora"),
                training_config.get("mode", "quick"),
                training_config.get("hyperparams", {}),
                training_config.get("gpu_class"),
            ],
            id=f"train-{project_id}",
        )
        model_id = train_result.model_id
        if not model_id:
            raise ApplicationError(
                f"Training for job {training_job_id} completed without creating a model record",
                non_retryable=True,
            )

        # Stage 4: Evaluate — create the evaluations row the suites update
        evaluation_id = await workflow.execute_activity(
            "create_evaluation",
            CreateEvaluationInput(tenant_id=tenant_id, model_id=model_id),
            start_to_close_timeout=timeouts.db_lookup(),
            retry_policy=RetryPolicy(maximum_attempts=3),
        )

        # Evaluation context: mode drives mode-specific suites (teacher
        # parity); the teacher goes along key-stripped — evaluation compares
        # against stored golden answers and never calls the teacher.
        teacher = training_config.get("teacher")
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
                training_config.get("mode", "quick"),
                (
                    {"teacher": {k: v for k, v in teacher.items() if k != "api_key"}}
                    if teacher
                    else None
                ),
                {
                    "mode": training_config.get("mode", "quick"),
                    "method": training_config.get("method", "qlora"),
                    "gpu_class": training_config.get("gpu_class"),
                },
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
                result_type=DeployModelOutput,
            )

        return {
            "project_id": project_id,
            "documents_processed": ingest_result["documents_processed"],
            "dataset_pairs": refine_result.pair_count,
            "training_job_id": training_job_id,
            "model_id": model_id,
            "evaluation_id": evaluation_id,
            "training_metrics": train_result.metrics,
            "eval_scores": eval_result.scores,
            "deployed": deploy_result is not None,
        }
