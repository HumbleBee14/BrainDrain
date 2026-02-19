"""Train workflow — runs a fine-tuning job.

Triggered when a user starts training from a prepared dataset.
"""

from datetime import timedelta

from temporalio import workflow

with workflow.unsafe.imports_passed_through():
    from src.activities.stubs import (
        StartTrainingInput,
        StartTrainingOutput,
        start_training,
    )


@workflow.defn
class TrainWorkflow:
    """Execute a fine-tuning training job.

    Input: tenant_id, training_job_id, dataset details, model config.
    Runs the training activity with extended timeout for GPU workloads.
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
        result = await workflow.execute_activity(
            start_training,
            StartTrainingInput(
                tenant_id=tenant_id,
                training_job_id=training_job_id,
                dataset_path=dataset_path,
                base_model=base_model,
                method=method,
                mode=mode,
                hyperparams=hyperparams,
                gpu_class=gpu_class,
            ),
            start_to_close_timeout=timedelta(hours=6),
            heartbeat_timeout=timedelta(minutes=5),
            retry_policy=workflow.RetryPolicy(maximum_attempts=1),
        )

        return result
