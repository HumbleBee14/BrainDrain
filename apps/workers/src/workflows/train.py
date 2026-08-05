"""Train workflow — mode dispatcher for fine-tuning jobs.

Routes to the appropriate child workflow based on training mode:
  - quick:     Direct activity call (single SFT round)
  - iterative: TrainIterativeWorkflow (loop + early stopping in workflow)
  - aligned:   TrainAlignedWorkflow (SFT → DPO, in-memory)
  - reasoning: TrainReasoningWorkflow (SFT → GRPO, in-memory)

FullPipelineWorkflow still calls TrainWorkflow.run — this dispatcher
is transparent to upstream callers.

A distill job whose teacher config carries an `extraction` block trains against
the teacher rather than only its text. That block is the only gate: without it,
every mode reaches training by the same path it always did. Which method it names
decides how the teacher is reached — `logit` runs one scoring pass up front and
trains on the stored distributions, while `on_policy` skips scoring entirely,
because the text it grades is written by the student during the run.
"""

from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ApplicationError

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.pipeline_records import (
        SetTeacherExtractionStatusInput,
        TeacherExtractionStatus,
    )
    from src.activities.stubs import (
        ExtractTeacherLogprobsInput,
        ExtractTeacherLogprobsOutput,
        FinalizeIterativeTrainingInput,
        StartTrainingInput,
        StartTrainingOutput,
    )
    from src.constants import (
        LOGIT_DISTILL_METHOD as LOGIT_METHOD,
    )
    from src.constants import (
        ON_POLICY_DISTILL_METHOD as ON_POLICY_METHOD,
    )
    from src.workflows.train_aligned import TrainAlignedWorkflow
    from src.workflows.train_iterative import TrainIterativeWorkflow
    from src.workflows.train_reasoning import TrainReasoningWorkflow

DISTILL_MODE = "distill"

TEACHER_ARTIFACTS_HYPERPARAM = "teacher_artifacts_prefix"
DISTILL_METHOD_HYPERPARAM = "distill_method"
TEACHER_MODEL_HYPERPARAM = "teacher_model"
TEACHER_REVISION_HYPERPARAM = "teacher_revision"
TEACHER_PRECISION_HYPERPARAM = "teacher_precision"

# Hyperparams the platform writes from an admitted plan, and nothing else may
# supply. Hyperparams are otherwise free-form and caller-controlled, so any key
# that names a model we will execute, or a storage prefix we will read, has to be
# on this list — see `borrowed_fidelity_keys`.
_PLATFORM_OWNED_HYPERPARAMS = (
    DISTILL_METHOD_HYPERPARAM,
    TEACHER_ARTIFACTS_HYPERPARAM,
    TEACHER_MODEL_HYPERPARAM,
    TEACHER_REVISION_HYPERPARAM,
    TEACHER_PRECISION_HYPERPARAM,
)


def extraction_plan(teacher_config: dict | None) -> dict | None:
    """The fidelity upgrade the API admitted for this job, or None.

    This is the single gate on the whole extra GPU pass. Every run that does not
    carry the block — which is every run of every mode that existed before
    high-fidelity distillation — must reach training by exactly the path it
    always did.
    """
    if not isinstance(teacher_config, dict):
        return None
    plan = teacher_config.get("extraction")
    return plan if isinstance(plan, dict) else None


def extraction_input(
    plan: dict,
    *,
    tenant_id: str,
    training_job_id: str,
    dataset_path: str,
    base_model: str,
) -> ExtractTeacherLogprobsInput:
    """Translate the admitted plan into the scoring activity's input.

    Two names differ across the seam and one value is easy to mistake for its
    neighbour: the plan's `top_k_logprobs` is the activity's `top_k`, and the
    plan's `gpu_class` is the class this *teacher* needs to run at all — never the
    class the student was asked to train on.
    """
    return ExtractTeacherLogprobsInput(
        tenant_id=tenant_id,
        training_job_id=training_job_id,
        dataset_path=dataset_path,
        teacher_model=plan["teacher_model"],
        teacher_revision=plan.get("teacher_revision", ""),
        student_model=base_model,
        precision=plan.get("precision", "bf16"),
        top_k=int(plan.get("top_k_logprobs", 32)),
        gpu_class=plan.get("gpu_class"),
    )


def hyperparams_with_artifacts(hyperparams: dict, artifact_prefix: str) -> dict:
    """Hyperparams that make the training activity pick the logit strategy.

    Both keys travel together on purpose: `distill_method` is what selects the
    strategy and `teacher_artifacts_prefix` is what that strategy refuses to run
    without, so neither can be present while the other is missing.
    """
    return {
        **hyperparams,
        DISTILL_METHOD_HYPERPARAM: LOGIT_METHOD,
        TEACHER_ARTIFACTS_HYPERPARAM: artifact_prefix,
    }


def borrowed_fidelity_keys(hyperparams: dict) -> list[str]:
    """Fidelity hyperparams that arrived from outside this workflow.

    Hyperparams are free-form and caller-supplied, but these keys are written here
    from the plan the API priced and admitted, and by nothing else. A copy arriving
    with the request would otherwise select a fidelity strategy — and name the S3
    prefix it reads a teacher's distributions from, or the model it boots on our
    own GPU — for a run that was never admitted and whose teacher may not be its
    own.
    """
    return sorted(key for key in _PLATFORM_OWNED_HYPERPARAMS if key in hyperparams)


def unsupported_plan_reason(plan: dict, mode: str) -> str | None:
    """Why this plan cannot be executed, or None if it can.

    Refusing here rather than at training time is what keeps a doomed run from
    paying a teacher-sized GPU bill first.
    """
    if mode != DISTILL_MODE:
        return f"A fidelity upgrade has no meaning for training mode '{mode}'"
    if plan.get("distill_method") not in (LOGIT_METHOD, ON_POLICY_METHOD):
        return f"Unsupported distillation method: {plan.get('distill_method')!r}"
    if not plan.get("teacher_model"):
        return "The fidelity plan names no teacher model"
    return None


def hyperparams_with_live_teacher(hyperparams: dict, plan: dict) -> dict:
    """Hyperparams that make the training activity pick the on-policy strategy.

    Unlike the logit path there is no scoring pass to run first: the teacher is
    started by the training container itself, so all that has to cross the seam is
    which teacher, pinned to which revision. The keys travel together for the same
    reason as the artifact prefix — the strategy refuses to run without a teacher,
    and the method is what selects the strategy.
    """
    resolved = {
        **hyperparams,
        DISTILL_METHOD_HYPERPARAM: ON_POLICY_METHOD,
        TEACHER_MODEL_HYPERPARAM: plan["teacher_model"],
    }
    if plan.get("teacher_revision"):
        resolved[TEACHER_REVISION_HYPERPARAM] = plan["teacher_revision"]
    if plan.get("precision"):
        resolved[TEACHER_PRECISION_HYPERPARAM] = plan["precision"]
    return resolved


@workflow.defn
class TrainWorkflow:
    """Dispatch training to the appropriate mode-specific workflow.

    Input: tenant_id, training_job_id, dataset details, model config, and the
    job's teacher config last. Temporal arguments are positional, so `teacher_config`
    is appended with a default and never inserted — a payload sent before it
    existed still binds every earlier argument to the same parameter.
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
        teacher_config: dict | None = None,
    ) -> StartTrainingOutput:
        workflow.set_current_details(f"Training mode: {mode}")

        borrowed = borrowed_fidelity_keys(hyperparams)
        if borrowed:
            raise ApplicationError(
                f"These hyperparams are set by the platform, not per job: {', '.join(borrowed)}",
                non_retryable=True,
            )

        plan = extraction_plan(teacher_config)
        if plan is not None:
            reason = unsupported_plan_reason(plan, mode)
            if reason:
                raise ApplicationError(reason, non_retryable=True)

            if plan["distill_method"] == ON_POLICY_METHOD:
                # No scoring pass: an on-policy teacher grades text that does not
                # exist yet, so it is started by the training container itself.
                hyperparams = hyperparams_with_live_teacher(hyperparams, plan)
            else:
                hyperparams = await self._score_with_teacher(
                    plan,
                    tenant_id=tenant_id,
                    training_job_id=training_job_id,
                    dataset_path=dataset_path,
                    base_model=base_model,
                    hyperparams=hyperparams,
                )

        if mode in ("quick", "distill"):
            # Direct activity — single SFT round, no multi-phase. Text distill is
            # the same pass over teacher-written data; the difference lives in
            # datagen (teacher answers) and evaluation (parity suite). A scored
            # run reaches the same activity and the hyperparams above route it to
            # the logit strategy.
            return await workflow.execute_activity(
                "start_training",
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
                f"Unknown training mode: {mode}. "
                "Valid modes: quick, distill, iterative, aligned, reasoning"
            )

    async def _score_with_teacher(
        self,
        plan: dict,
        *,
        tenant_id: str,
        training_job_id: str,
        dataset_path: str,
        base_model: str,
        hyperparams: dict,
    ) -> dict:
        """Score the dataset with the teacher, then point training at the result.

        Cancellation arrives as `CancelledError`, which the failure path below
        deliberately does not catch: a cancelled extraction stays RUNNING, which is
        the fact that tells an operator (and the orphan sweep) the GPU still owed
        money was the teacher's and was never given a chance to stop on its own.
        Its charge is not lost with it — the RUNNING transition reserved one, and
        the API's outbox relay bills that reservation at the admitted estimate once
        the pass cannot plausibly still be running.

        The terminal transitions differ in what they can pay for: a completed pass
        hands over the runtimes it measured, a failed one has no result to hand
        over and is billed from how long its reservation stood.
        """
        workflow.set_current_details(f"Scoring with teacher {plan['teacher_model']}")
        await self._set_extraction_status(
            tenant_id, training_job_id, TeacherExtractionStatus.RUNNING
        )
        try:
            result = await workflow.execute_activity(
                "extract_teacher_logprobs",
                extraction_input(
                    plan,
                    tenant_id=tenant_id,
                    training_job_id=training_job_id,
                    dataset_path=dataset_path,
                    base_model=base_model,
                ),
                task_queue="ml-pipeline-gpu",
                start_to_close_timeout=timeouts.teacher_extraction_activity(),
                heartbeat_timeout=timeouts.teacher_extraction_heartbeat(),
                retry_policy=RetryPolicy(maximum_attempts=2),
                result_type=ExtractTeacherLogprobsOutput,
            )
        except Exception:
            await self._set_extraction_status(
                tenant_id, training_job_id, TeacherExtractionStatus.FAILED
            )
            raise

        await self._set_extraction_status(
            tenant_id,
            training_job_id,
            TeacherExtractionStatus.COMPLETED,
            metrics=result.metrics,
        )
        workflow.logger.info(
            "Teacher scored %d records (%d positions) for job %s",
            result.records,
            result.scored_positions,
            training_job_id,
        )
        return hyperparams_with_artifacts(hyperparams, result.artifact_prefix)

    async def _set_extraction_status(
        self, tenant_id: str, job_id: str, status: str, metrics: dict | None = None
    ) -> None:
        await workflow.execute_activity(
            "set_teacher_extraction_status",
            SetTeacherExtractionStatusInput(
                tenant_id=tenant_id, training_job_id=job_id, status=status, metrics=metrics
            ),
            start_to_close_timeout=timeouts.db_lookup(),
            retry_policy=RetryPolicy(maximum_attempts=3),
        )
