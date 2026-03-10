"""Aligned training workflow — SFT → DPO in a single activity.

Wraps the existing start_training activity with mode="aligned" in a child
workflow for observability. The model stays in GPU memory between SFT and
DPO phases (no checkpoint save/load overhead).

Future: if retry isolation between SFT and DPO becomes critical,
split into two activities here without changing the dispatcher.
"""



from temporalio import workflow

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.stubs import StartTrainingInput, StartTrainingOutput


@workflow.defn
class TrainAlignedWorkflow:
    """SFT → DPO alignment training.

    Delegates to the existing start_training activity with mode="aligned".
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
        workflow.set_current_details("Aligned training: SFT → DPO")

        result = await workflow.execute_activity(
            "start_training",
            StartTrainingInput(
                tenant_id=tenant_id,
                training_job_id=training_job_id,
                dataset_path=dataset_path,
                base_model=base_model,
                method=method,
                mode="aligned",
                hyperparams=hyperparams,
                gpu_class=gpu_class,
            ),
            task_queue="ml-pipeline-gpu",
            start_to_close_timeout=timeouts.train_activity(),
            heartbeat_timeout=timeouts.train_heartbeat(),
            retry_policy=workflow.RetryPolicy(maximum_attempts=1),
        )

        workflow.set_current_details("Aligned training complete")
        return result
