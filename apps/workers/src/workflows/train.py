"""Train workflow — mode dispatcher for fine-tuning jobs.

Routes to the appropriate child workflow based on training mode:
  - quick:     Direct activity call (single SFT round)
  - iterative: TrainIterativeWorkflow (loop + early stopping in workflow)
  - aligned:   TrainAlignedWorkflow (SFT → DPO, in-memory)
  - reasoning: TrainReasoningWorkflow (SFT → GRPO, in-memory)

FullPipelineWorkflow still calls TrainWorkflow.run — this dispatcher
is transparent to upstream callers.
"""

from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ApplicationError

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.stubs import (
        FinalizeIterativeTrainingInput,
        StartTrainingInput,
        StartTrainingOutput,
    )
    from src.workflows.train_aligned import TrainAlignedWorkflow
    from src.workflows.train_iterative import TrainIterativeWorkflow
    from src.workflows.train_reasoning import TrainReasoningWorkflow


@workflow.defn
class TrainWorkflow:
    """Dispatch training to the appropriate mode-specific workflow.

    Input unchanged: tenant_id, training_job_id, dataset details, model config.
    Output unchanged: StartTrainingOutput (adapter_path, size, metrics).
    """

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        training_job_id: str,
        dataset_path: str,
        base_model: str,
        method: str,
        mode: str,
        hyperparams: dict,
        gpu_class: str | None = None,
    ) -> StartTrainingOutput:
        workflow.set_current_details(f"Training mode: {mode}")

        if mode == "quick":
            # Direct activity — single SFT round, no multi-phase
            return await workflow.execute_activity(
                "start_training",
                StartTrainingInput(
                    tenant_id=tenant_id,
                    training_job_id=training_job_id,
                    dataset_path=dataset_path,
                    base_model=base_model,
                    method=method,
                    mode="quick",
                    hyperparams=hyperparams,
                    gpu_class=gpu_class,
                ),
                task_queue="ml-pipeline-gpu",
                start_to_close_timeout=timeouts.train_activity(),
                heartbeat_timeout=timeouts.train_heartbeat(),
                retry_policy=RetryPolicy(maximum_attempts=2),
                result_type=StartTrainingOutput,
            )

        elif mode == "iterative":
            result = await workflow.execute_child_workflow(
                TrainIterativeWorkflow.run,
                args=[
                    tenant_id,
                    training_job_id,
                    dataset_path,
                    base_model,
                    method,
                    hyperparams,
                    gpu_class,
                ],
                id=f"train-iterative-{training_job_id}",
            )

            # Finalize: update DB status, calculate cost, create model record
            model_id = await workflow.execute_activity(
                "finalize_iterative_training",
                FinalizeIterativeTrainingInput(
                    tenant_id=tenant_id,
                    training_job_id=training_job_id,
                    base_model=base_model,
                    mode="iterative",
                    adapter_path=result.adapter_path,
                    adapter_size_bytes=result.adapter_size_bytes,
                    metrics=result.metrics,
                    gpu_class=gpu_class,
                ),
                task_queue="ml-pipeline-gpu",
                start_to_close_timeout=timedelta(minutes=5),
                retry_policy=RetryPolicy(maximum_attempts=3),
            )

            result.model_id = model_id or ""
            return result

        elif mode == "aligned":
            return await workflow.execute_child_workflow(
                TrainAlignedWorkflow.run,
                args=[
                    tenant_id,
                    training_job_id,
                    dataset_path,
                    base_model,
                    method,
                    hyperparams,
                    gpu_class,
                ],
                id=f"train-aligned-{training_job_id}",
            )

        elif mode == "reasoning":
            return await workflow.execute_child_workflow(
                TrainReasoningWorkflow.run,
                args=[
                    tenant_id,
                    training_job_id,
                    dataset_path,
                    base_model,
                    method,
                    hyperparams,
                    gpu_class,
                ],
                id=f"train-reasoning-{training_job_id}",
            )

        else:
            raise ApplicationError(
                f"Unknown training mode: {mode}. Valid modes: quick, iterative, aligned, reasoning"
            )
