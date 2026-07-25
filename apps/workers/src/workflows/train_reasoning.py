"""Reasoning training workflow — SFT → GRPO in a single activity.

Wraps the existing start_training activity with mode="reasoning" in a child
workflow for observability. The model stays in GPU memory between SFT and
GRPO phases (no checkpoint save/load overhead).

Future: if retry isolation between SFT and GRPO becomes critical,
split into two activities here without changing the dispatcher.
"""

from temporalio import workflow
from temporalio.common import RetryPolicy

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.stubs import StartTrainingInput, StartTrainingOutput


@workflow.defn
class TrainReasoningWorkflow:
    """SFT → GRPO reasoning optimization.

    Delegates to the existing start_training activity with mode="reasoning".
    Provides its own workflow ID for Temporal UI visibility and future splitting.
    """

    @workflow.run
    async def run(
        self,
        tenant_id: str,
        training_job_id: str,
        dataset_path: str,
        base_model: str,
        method: str,
        hyperparams: dict,
        gpu_class: str | None = None,
    ) -> StartTrainingOutput:
        workflow.set_current_details("Reasoning training: SFT → GRPO")

        result = await workflow.execute_activity(
            "start_training",
            StartTrainingInput(
                tenant_id=tenant_id,
                training_job_id=training_job_id,
                dataset_path=dataset_path,
                base_model=base_model,
                method=method,
                mode="reasoning",
                hyperparams=hyperparams,
                gpu_class=gpu_class,
            ),
            task_queue="ml-pipeline-gpu",
            start_to_close_timeout=timeouts.train_activity(),
            heartbeat_timeout=timeouts.train_heartbeat(),
            retry_policy=RetryPolicy(maximum_attempts=2),
            result_type=StartTrainingOutput,
        )

        workflow.set_current_details("Reasoning training complete")
        return result
