"""Iterative training workflow — multiple SFT rounds with early stopping.

Extracts the iteration loop from the monolithic training activity into a
proper Temporal workflow. Each iteration is a pair of activities:
  1. train_sft_round — one SFT pass, saves adapter checkpoint
  2. evaluate_holdout — validation eval on held-out data

The workflow manages the loop, early stopping (eval_loss regression),
progress tracking (signals/queries), and Temporal UI visibility.
"""

import math

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ApplicationError

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.stubs import (
        EvaluateHoldoutInput,
        EvaluateHoldoutOutput,
        StartTrainingOutput,
        TrainSftRoundInput,
        TrainSftRoundOutput,
    )


# -- Best-checkpoint selection (pure helpers, unit-tested without Temporal) --


def effective_eval_loss(raw_eval_loss, *, eval_failed: bool) -> tuple[float, float | None]:
    """Map a round's holdout-eval outcome to ``(comparison_loss, recorded_loss)``.

    ``comparison_loss`` drives best-checkpoint selection: the finite eval loss,
    or ``+inf`` when the round must be ineligible (eval failed, or produced a
    non-finite/invalid value). ``recorded_loss`` is what we persist in metrics:
    the finite value, or ``None`` when there is no valid eval loss. Training loss
    is never substituted — it is systematically lower than eval loss and would
    let a failed round win "best".
    """
    if eval_failed:
        return float("inf"), None
    # bool is an int subclass; exclude it explicitly.
    if isinstance(raw_eval_loss, bool) or not isinstance(raw_eval_loss, int | float):
        return float("inf"), None
    if not math.isfinite(raw_eval_loss):
        return float("inf"), None
    value = float(raw_eval_loss)
    return value, value


def is_meaningful_improvement(candidate: float, incumbent: float, min_delta: float) -> bool:
    """True if ``candidate`` beats ``incumbent`` by more than ``min_delta``.

    Only a finite candidate can improve on the incumbent, so an ineligible
    (``+inf``) round never becomes best.
    """
    return math.isfinite(candidate) and candidate < (incumbent - min_delta)


def resolve_final_checkpoint(
    best_path: str, best_size: int, last_path: str, last_size: int
) -> tuple[str, int, str | None] | None:
    """Choose the checkpoint to ship after all rounds.

    Returns ``(adapter_path, adapter_size, fallback_reason)``: ``fallback_reason``
    is ``None`` for a normally-selected best, or ``"last_trained_adapter"`` when
    no round was successfully evaluated but at least one trained (so we never
    ship an empty path). Returns ``None`` when not even one round trained — the
    caller must then fail loudly.
    """
    if best_path:
        return best_path, best_size, None
    if last_path:
        return last_path, last_size, "last_trained_adapter"
    return None


@workflow.defn
class TrainIterativeWorkflow:
    """Multi-round SFT training with holdout evaluation and early stopping.

    Each iteration: train_sft_round → evaluate_holdout → decision.
    Stops early if eval_loss regresses or a signal is received.
    """

    def __init__(self) -> None:
        self._early_stop_requested = False
        self._current_iteration = 0
        self._total_iterations = 0
        self._best_eval_loss = float("inf")
        self._best_adapter_path = ""
        self._iteration_metrics: dict = {}

    @workflow.signal
    async def request_early_stop(self) -> None:
        """Signal from user or external system to stop after current iteration."""
        self._early_stop_requested = True
        workflow.logger.info("Early stop requested, will stop after current iteration")

    @workflow.query
    def get_progress(self) -> dict:
        """Query current training progress."""
        return {
            "current_iteration": self._current_iteration,
            "total_iterations": self._total_iterations,
            "best_eval_loss": self._best_eval_loss,
            "best_adapter_path": self._best_adapter_path,
            "early_stop_requested": self._early_stop_requested,
            "iteration_metrics": self._iteration_metrics,
        }

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
        num_iterations = hyperparams.get("num_iterations", 3)
        patience = hyperparams.get("early_stop_patience", 2)
        min_delta = hyperparams.get("early_stop_min_delta", 0.01)
        self._total_iterations = num_iterations

        workflow.logger.info(
            "Starting iterative training: %d iterations, patience=%d, min_delta=%.4f for job %s",
            num_iterations,
            patience,
            min_delta,
            training_job_id,
        )

        previous_adapter_path: str | None = None
        best_adapter_path: str = ""
        best_adapter_size: int = 0
        # Most recent successfully-trained adapter, used as a last-resort fallback
        # if every round's holdout eval fails (so we never ship an empty path).
        last_adapter_path: str = ""
        last_adapter_size: int = 0
        no_improvement_count: int = 0
        all_metrics: dict = {}

        for iteration in range(num_iterations):
            self._current_iteration = iteration + 1
            workflow.set_current_details(
                f"Iteration {iteration + 1}/{num_iterations} "
                f"(best eval_loss: {self._best_eval_loss:.4f})"
                if self._best_eval_loss < float("inf")
                else f"Iteration {iteration + 1}/{num_iterations}"
            )

            # Check early stop signal before starting new iteration
            if self._early_stop_requested:
                workflow.logger.info(
                    "Early stop: halting at iteration %d/%d",
                    iteration,
                    num_iterations,
                )
                all_metrics["early_stopped"] = True
                all_metrics["early_stop_reason"] = "user_signal"
                break

            # Activity 1: Train one SFT round
            sft_result: TrainSftRoundOutput = await workflow.execute_activity(
                "train_sft_round",
                TrainSftRoundInput(
                    tenant_id=tenant_id,
                    training_job_id=training_job_id,
                    dataset_path=dataset_path,
                    base_model=base_model,
                    method=method,
                    hyperparams=hyperparams,
                    iteration=iteration,
                    adapter_path=previous_adapter_path,
                    gpu_class=gpu_class,
                ),
                task_queue="ml-pipeline-gpu",
                start_to_close_timeout=timeouts.train_iterative_activity(),
                heartbeat_timeout=timeouts.train_heartbeat(),
                retry_policy=RetryPolicy(maximum_attempts=2),
            )

            all_metrics[f"iter_{iteration}"] = sft_result.metrics

            # Activity 2: Evaluate on holdout set
            try:
                eval_result: EvaluateHoldoutOutput = await workflow.execute_activity(
                    "evaluate_holdout",
                    EvaluateHoldoutInput(
                        tenant_id=tenant_id,
                        training_job_id=training_job_id,
                        adapter_path=sft_result.adapter_path,
                        base_model=base_model,
                        method=method,
                        dataset_path=dataset_path,
                        hyperparams=hyperparams,
                        iteration=iteration,
                        gpu_class=gpu_class,
                    ),
                    task_queue="ml-pipeline-gpu",
                    start_to_close_timeout=timeouts.holdout_eval_activity(),
                    heartbeat_timeout=timeouts.holdout_eval_heartbeat(),
                    retry_policy=RetryPolicy(maximum_attempts=2),
                )
                raw_eval_loss = eval_result.eval_loss
                all_metrics[f"iter_{iteration}_eval"] = eval_result.metrics
                # A failed/invalid holdout eval must NOT let this round become
                # "best": training loss is systematically lower than eval loss,
                # so substituting it would let a failed round beat genuinely
                # evaluated ones. effective_eval_loss maps such rounds to +inf.
                eval_loss, recorded_loss = effective_eval_loss(raw_eval_loss, eval_failed=False)
            except Exception as e:
                workflow.logger.warning(
                    "Holdout eval failed for iteration %d: %s. "
                    "Round is ineligible to become best checkpoint.",
                    iteration,
                    str(e),
                )
                eval_loss, recorded_loss = effective_eval_loss(None, eval_failed=True)
                all_metrics[f"iter_{iteration}_eval_failed"] = str(e)[:200]

            all_metrics[f"iter_{iteration}_eval_loss"] = recorded_loss

            # Remember the most recent successfully-trained adapter as a fallback.
            last_adapter_path = sft_result.adapter_path
            last_adapter_size = sft_result.adapter_size_bytes

            # Track best (with min_delta threshold for meaningful improvement).
            # A round only becomes best if it was successfully evaluated with a
            # finite eval_loss that meaningfully beats the incumbent.
            improved = is_meaningful_improvement(eval_loss, self._best_eval_loss, min_delta)
            if improved:
                self._best_eval_loss = eval_loss
                self._best_adapter_path = sft_result.adapter_path
                best_adapter_path = sft_result.adapter_path
                best_adapter_size = sft_result.adapter_size_bytes
                no_improvement_count = 0
            else:
                no_improvement_count += 1

            self._iteration_metrics[f"iter_{iteration}"] = {
                "train_metrics": sft_result.metrics,
                "eval_loss": recorded_loss,
                "improved": improved,
                "no_improvement_count": no_improvement_count,
            }

            # Early stopping: no meaningful improvement for `patience` consecutive iterations
            if iteration > 0 and no_improvement_count >= patience:
                workflow.logger.info(
                    "Early stop: no improvement for %d iterations "
                    "(best=%.4f, current=%.4f, min_delta=%.4f) at iteration %d",
                    patience,
                    self._best_eval_loss,
                    eval_loss,
                    min_delta,
                    iteration + 1,
                )
                all_metrics["early_stopped"] = True
                all_metrics["early_stop_reason"] = "no_improvement"
                all_metrics["early_stop_patience"] = patience
                all_metrics["early_stop_min_delta"] = min_delta
                break

            previous_adapter_path = sft_result.adapter_path

        # No round produced a valid best checkpoint (e.g. every holdout eval
        # failed). Rather than silently returning an empty adapter path, fall
        # back to the last successfully-trained adapter — or fail loudly if not
        # even one round trained.
        resolved = resolve_final_checkpoint(
            best_adapter_path, best_adapter_size, last_adapter_path, last_adapter_size
        )
        if resolved is None:
            raise ApplicationError(
                "Iterative training produced no usable checkpoint: no round trained successfully."
            )
        best_adapter_path, best_adapter_size, fallback_reason = resolved
        if fallback_reason:
            workflow.logger.warning(
                "No round was successfully evaluated; falling back to the "
                "last successfully-trained adapter: %s",
                best_adapter_path,
            )
            self._best_adapter_path = best_adapter_path
            all_metrics["best_selection_fallback"] = fallback_reason

        all_metrics["total_iterations"] = self._current_iteration
        all_metrics["best_eval_loss"] = self._best_eval_loss
        all_metrics["patience"] = patience
        all_metrics["min_delta"] = min_delta
        all_metrics["best_iteration"] = next(
            (
                k
                for k, v in self._iteration_metrics.items()
                if isinstance(v, dict) and v.get("eval_loss") == self._best_eval_loss
            ),
            "iter_0",
        )

        workflow.set_current_details(
            f"Complete: {self._current_iteration} iterations, "
            f"best eval_loss: {self._best_eval_loss:.4f}"
        )

        return StartTrainingOutput(
            adapter_path=best_adapter_path,
            adapter_size_bytes=best_adapter_size,
            metrics=all_metrics,
        )
