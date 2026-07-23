"""GPU compute provider abstraction for training and evaluation jobs.

Provides a GpuProvider protocol with pluggable implementations:
  - LocalGpuProvider: Runs GPU work on the worker's local GPU (dev/testing)
  - ModalGpuProvider: Sends GPU work to Modal serverless GPUs (production)

The provider is selected by the APP_GPU_PROVIDER config:
  - "local" (default): Use the worker's own GPU
  - "modal": Provision ephemeral GPU on Modal

Every provider exposes one method per GPU-bound activity — run_training,
run_sft_round, run_evaluate_holdout, run_evaluation — so the worker-side
activity keeps all DB/Redis writes and the provider owns only the compute
(local in-process, or a remote Modal FunctionCall with a durable reservation).

Adding a new provider (e.g., RunPod):
  1. Create a class implementing GpuProvider
  2. Add it to create_gpu_provider()
  3. Add config vars to WorkerSettings
"""

import logging
from typing import Protocol

logger = logging.getLogger("platform.gpu")


class GpuProvider(Protocol):
    """Protocol for GPU compute providers.

    Each method runs one GPU-bound unit of work end to end (download data →
    load model → compute → upload result) and returns a plain result dict.
    """

    async def run_training(
        self,
        *,
        tenant_id: str,
        training_job_id: str,
        dataset_path: str,
        base_model: str,
        method: str,
        mode: str,
        hyperparams: dict,
        gpu_class: str | None,
        llm_config: dict,
    ) -> dict:
        """Execute a single-shot training job.

        Returns: dict with keys adapter_path, adapter_size_bytes, metrics.
        """
        ...

    async def run_sft_round(
        self,
        *,
        tenant_id: str,
        training_job_id: str,
        dataset_path: str,
        base_model: str,
        method: str,
        hyperparams: dict,
        iteration: int,
        adapter_path: str | None,
        gpu_class: str | None,
    ) -> dict:
        """Execute one SFT round of the iterative workflow.

        Returns: dict with keys adapter_path, adapter_size_bytes, metrics.
        """
        ...

    async def run_evaluate_holdout(
        self,
        *,
        tenant_id: str,
        training_job_id: str,
        adapter_path: str,
        base_model: str,
        method: str,
        dataset_path: str,
        hyperparams: dict,
        iteration: int,
        gpu_class: str | None,
    ) -> dict:
        """Evaluate one iteration's adapter on the holdout split.

        Returns: dict with keys eval_loss, metrics.
        """
        ...

    async def run_evaluation(
        self,
        *,
        tenant_id: str,
        model_id: str,
        evaluation_id: str,
        adapter_path: str,
        base_model: str,
        dataset_path: str,
        judge_model: str,
        judge_api_base: str,
        gpu_class: str | None,
        llm_config: dict,
    ) -> dict:
        """Run the full evaluation suite on a fine-tuned model.

        Returns: dict with keys scores, report.
        """
        ...


class LocalGpuProvider:
    """Run GPU work on the local worker's GPU.

    This is the default provider for development and single-machine setups.
    The worker process must have access to a CUDA-capable GPU. Each method
    calls the same pure-compute core used by the remote path — same behavior
    as before the provider abstraction.
    """

    def __init__(self, infra):
        self.infra = infra

    async def run_training(
        self,
        *,
        tenant_id: str,
        training_job_id: str,
        dataset_path: str,
        base_model: str,
        method: str,
        mode: str,
        hyperparams: dict,
        gpu_class: str | None,
        llm_config: dict,
    ) -> dict:
        logger.info(
            "Running training locally (job=%s, model=%s, gpu_class=%s)",
            training_job_id[:8],
            base_model,
            gpu_class or "local",
        )

        # Import here to avoid loading heavy ML deps when using Modal provider
        from src.activities.stubs import StartTrainingInput
        from src.activities.train_model import run_training_core
        from src.tenant_config import TenantLlmConfig

        input_data = StartTrainingInput(
            tenant_id=tenant_id,
            training_job_id=training_job_id,
            dataset_path=dataset_path,
            base_model=base_model,
            method=method,
            mode=mode,
            hyperparams=hyperparams,
            gpu_class=gpu_class,
        )
        result = await run_training_core(
            input_data,
            s3=self.infra.s3,
            s3_bucket=self.infra.s3_bucket,
            settings=self.infra.settings,
            llm_config=TenantLlmConfig(**llm_config),
        )
        return {
            "adapter_path": result.adapter_path,
            "adapter_size_bytes": result.adapter_size_bytes,
            "metrics": result.metrics,
        }

    async def run_sft_round(
        self,
        *,
        tenant_id: str,
        training_job_id: str,
        dataset_path: str,
        base_model: str,
        method: str,
        hyperparams: dict,
        iteration: int,
        adapter_path: str | None,
        gpu_class: str | None,
    ) -> dict:
        logger.info(
            "Running SFT round locally (job=%s, iter=%d, model=%s)",
            training_job_id[:8],
            iteration,
            base_model,
        )

        from src.activities.stubs import TrainSftRoundInput
        from src.activities.train_model import run_sft_round_core

        input_data = TrainSftRoundInput(
            tenant_id=tenant_id,
            training_job_id=training_job_id,
            dataset_path=dataset_path,
            base_model=base_model,
            method=method,
            hyperparams=hyperparams,
            iteration=iteration,
            adapter_path=adapter_path,
            gpu_class=gpu_class,
        )
        result = await run_sft_round_core(
            input_data,
            s3=self.infra.s3,
            s3_bucket=self.infra.s3_bucket,
            settings=self.infra.settings,
        )
        return {
            "adapter_path": result.adapter_path,
            "adapter_size_bytes": result.adapter_size_bytes,
            "metrics": result.metrics,
        }

    async def run_evaluate_holdout(
        self,
        *,
        tenant_id: str,
        training_job_id: str,
        adapter_path: str,
        base_model: str,
        method: str,
        dataset_path: str,
        hyperparams: dict,
        iteration: int,
        gpu_class: str | None,
    ) -> dict:
        logger.info(
            "Running holdout eval locally (job=%s, iter=%d)", training_job_id[:8], iteration
        )

        from src.activities.stubs import EvaluateHoldoutInput
        from src.activities.train_model import run_evaluate_holdout_core

        input_data = EvaluateHoldoutInput(
            tenant_id=tenant_id,
            training_job_id=training_job_id,
            adapter_path=adapter_path,
            base_model=base_model,
            method=method,
            dataset_path=dataset_path,
            hyperparams=hyperparams,
            iteration=iteration,
            gpu_class=gpu_class,
        )
        result = await run_evaluate_holdout_core(
            input_data,
            s3=self.infra.s3,
            s3_bucket=self.infra.s3_bucket,
            settings=self.infra.settings,
        )
        return {"eval_loss": result.eval_loss, "metrics": result.metrics}

    async def run_evaluation(
        self,
        *,
        tenant_id: str,
        model_id: str,
        evaluation_id: str,
        adapter_path: str,
        base_model: str,
        dataset_path: str,
        judge_model: str,
        judge_api_base: str,
        gpu_class: str | None,
        llm_config: dict,
    ) -> dict:
        logger.info("Running evaluation locally (eval=%s, model=%s)", evaluation_id[:8], base_model)

        from src.activities.run_evaluation import run_evaluation_core
        from src.activities.stubs import RunEvaluationInput
        from src.tenant_config import TenantLlmConfig

        input_data = RunEvaluationInput(
            tenant_id=tenant_id,
            model_id=model_id,
            evaluation_id=evaluation_id,
            adapter_path=adapter_path,
            base_model=base_model,
            dataset_path=dataset_path,
            judge_model=judge_model,
            judge_api_base=judge_api_base,
            gpu_class=gpu_class,
        )
        result = await run_evaluation_core(
            input_data,
            s3=self.infra.s3,
            s3_bucket=self.infra.s3_bucket,
            settings=self.infra.settings,
            llm_config=TenantLlmConfig(**llm_config),
        )
        return {"scores": result.scores, "report": result.report}


# Reservation tables are internal constants (never user input) — safe to
# interpolate into the reservation SQL below.
_RESERVATION_TABLES = ("training_jobs", "evaluations")


class ModalGpuProvider:
    """Run GPU work on Modal serverless GPUs via a pre-deployed app.

    Invokes the deployed functions (see apps/workers/modal_app.py) with
    spawn/poll so the worker event loop never blocks. Persists the Modal
    FunctionCall id to a reservation column BEFORE polling, so an activity
    retry / worker restart recovers the in-flight job instead of launching
    (and paying for) a duplicate GPU run.

    Reservation columns:
      - single-shot training + iterative rounds/eval: training_jobs.modal_call_id
      - full evaluation suite:                        evaluations.modal_call_id

    Iterative work runs its rounds sequentially (one call in flight at a
    time), so the single training_jobs.modal_call_id column is reused across
    rounds — but each round CLEARS it after completion so the next round does
    not recover a finished call. Single-shot training never clears (one call).

    The remote functions are pure-compute; this provider (worker-side) owns
    every reservation DB write.
    """

    def __init__(self, infra):
        self.infra = infra
        self._validate_modal_available()

    def _validate_modal_available(self):
        try:
            import modal  # noqa: F401
        except ImportError as e:
            raise RuntimeError(
                "Modal is not installed. Install the gpu-cloud extra: uv sync --extra gpu-cloud"
            ) from e

    def _resolve_gpu(self, gpu_class: str | None) -> str:
        from src.constants import MODAL_DEFAULT_GPU, MODAL_GPU_MAP

        return MODAL_GPU_MAP.get((gpu_class or "").lower(), MODAL_DEFAULT_GPU)

    def _recoverable_call_id(self, stored: str | None, function_name: str) -> str | None:
        """Return the FunctionCall id to recover, or None to spawn fresh.

        Reservations are stored tagged "<function_name>:<call_id>". Recover only
        when the tag matches the current function, so a stale reservation from a
        different function on a shared column is never reconnected (it will be
        overwritten by a fresh spawn instead). A bare (untagged) value is a
        legacy single-shot-training reservation and is recoverable only by the
        training function, preserving crash-recovery across the upgrade deploy.
        """
        if not stored:
            return None
        tag, sep, call_id = stored.partition(":")
        if sep:
            return call_id if tag == function_name else None
        # Legacy bare id (pre-tagging release): only the training function owns it.
        if function_name == self.infra.settings.modal_function_name:
            return stored
        return None

    async def _run_remote(
        self,
        *,
        function_name: str,
        payload: dict,
        gpu: str,
        table: str,
        row_id: str,
        tenant_id: str,
        label: str,
        clear_after: bool,
    ) -> dict:
        """Spawn (or recover) a remote FunctionCall, poll it, return its result.

        Reservation flow (durable, no duplicate GPU spend on retry):
          1. If a modal_call_id is stored for this row AND it belongs to THIS
             function, recover it (retry / worker-restart reconnect).
          2. Otherwise spawn, then persist the call id BEFORE polling.
          3. Poll without blocking the event loop (heartbeat between waits).
          4. If clear_after, null the reservation so a later same-function call
             on the same row (next iterative round) does not recover a finished
             call.

        The stored value is tagged "<function_name>:<call_id>" so a reservation
        left behind by one remote function (e.g. a tolerated holdout-eval
        failure that never cleared) can NEVER be recovered by a different
        function (the next round's train_sft_round) on the same shared
        training_jobs.modal_call_id column — which would otherwise return the
        wrong result / skip training. Bare (untagged) ids written by the
        earlier single-shot-training release are treated as legacy training
        reservations for backward compatibility across deploys.
        """
        import asyncio

        import modal
        from temporalio import activity

        if table not in _RESERVATION_TABLES:  # defensive: table is an internal literal
            raise ValueError(f"Unknown reservation table: {table}")

        settings = self.infra.settings
        db = self.infra.db

        stored = await db.fetchval(
            f"SELECT modal_call_id FROM {table} WHERE id = $1 AND tenant_id = $2",  # noqa: S608
            row_id,
            tenant_id,
        )

        recover_id = self._recoverable_call_id(stored, function_name)
        if recover_id:
            logger.info("Recovering Modal call %s for %s %s", recover_id, label, row_id[:8])
            fc = modal.FunctionCall.from_id(recover_id)
        else:
            fn = modal.Function.from_name(settings.modal_app_name, function_name)
            logger.info("Spawning Modal %s (row=%s, gpu=%s)", label, row_id[:8], gpu)
            fc = await fn.with_options(gpu=gpu).spawn.aio(payload)
            # A stale, mismatched reservation (if any) is overwritten here — safe,
            # because at most one remote call is ever in flight per row.
            await db.execute(
                f"UPDATE {table} SET modal_call_id = $1 WHERE id = $2 AND tenant_id = $3",  # noqa: S608
                f"{function_name}:{fc.object_id}",
                row_id,
                tenant_id,
            )

        while True:
            try:
                result = await fc.get.aio(timeout=0)
                break
            except TimeoutError:
                activity.heartbeat()
                await asyncio.sleep(settings.modal_poll_interval_secs)

        if clear_after:
            await db.execute(
                f"UPDATE {table} SET modal_call_id = NULL WHERE id = $1 AND tenant_id = $2",  # noqa: S608
                row_id,
                tenant_id,
            )

        logger.info("Modal %s complete (row=%s)", label, row_id[:8])
        return result

    async def run_training(
        self,
        *,
        tenant_id: str,
        training_job_id: str,
        dataset_path: str,
        base_model: str,
        method: str,
        mode: str,
        hyperparams: dict,
        gpu_class: str | None,
        llm_config: dict,
    ) -> dict:
        payload = {
            "input": {
                "tenant_id": tenant_id,
                "training_job_id": training_job_id,
                "dataset_path": dataset_path,
                "base_model": base_model,
                "method": method,
                "mode": mode,
                "hyperparams": hyperparams,
                "gpu_class": gpu_class,
            },
            "llm_config": llm_config,
        }
        return await self._run_remote(
            function_name=self.infra.settings.modal_function_name,
            payload=payload,
            gpu=self._resolve_gpu(gpu_class),
            table="training_jobs",
            row_id=training_job_id,
            tenant_id=tenant_id,
            label="training",
            clear_after=False,
        )

    async def run_sft_round(
        self,
        *,
        tenant_id: str,
        training_job_id: str,
        dataset_path: str,
        base_model: str,
        method: str,
        hyperparams: dict,
        iteration: int,
        adapter_path: str | None,
        gpu_class: str | None,
    ) -> dict:
        payload = {
            "input": {
                "tenant_id": tenant_id,
                "training_job_id": training_job_id,
                "dataset_path": dataset_path,
                "base_model": base_model,
                "method": method,
                "hyperparams": hyperparams,
                "iteration": iteration,
                "adapter_path": adapter_path,
                "gpu_class": gpu_class,
            }
        }
        return await self._run_remote(
            function_name=self.infra.settings.modal_sft_round_function_name,
            payload=payload,
            gpu=self._resolve_gpu(gpu_class),
            table="training_jobs",
            row_id=training_job_id,
            tenant_id=tenant_id,
            label=f"sft_round(iter={iteration})",
            clear_after=True,
        )

    async def run_evaluate_holdout(
        self,
        *,
        tenant_id: str,
        training_job_id: str,
        adapter_path: str,
        base_model: str,
        method: str,
        dataset_path: str,
        hyperparams: dict,
        iteration: int,
        gpu_class: str | None,
    ) -> dict:
        payload = {
            "input": {
                "tenant_id": tenant_id,
                "training_job_id": training_job_id,
                "adapter_path": adapter_path,
                "base_model": base_model,
                "method": method,
                "dataset_path": dataset_path,
                "hyperparams": hyperparams,
                "iteration": iteration,
                "gpu_class": gpu_class,
            }
        }
        return await self._run_remote(
            function_name=self.infra.settings.modal_evaluate_holdout_function_name,
            payload=payload,
            gpu=self._resolve_gpu(gpu_class),
            table="training_jobs",
            row_id=training_job_id,
            tenant_id=tenant_id,
            label=f"holdout_eval(iter={iteration})",
            clear_after=True,
        )

    async def run_evaluation(
        self,
        *,
        tenant_id: str,
        model_id: str,
        evaluation_id: str,
        adapter_path: str,
        base_model: str,
        dataset_path: str,
        judge_model: str,
        judge_api_base: str,
        gpu_class: str | None,
        llm_config: dict,
    ) -> dict:
        payload = {
            "input": {
                "tenant_id": tenant_id,
                "model_id": model_id,
                "evaluation_id": evaluation_id,
                "adapter_path": adapter_path,
                "base_model": base_model,
                "dataset_path": dataset_path,
                "judge_model": judge_model,
                "judge_api_base": judge_api_base,
                "gpu_class": gpu_class,
            },
            "llm_config": llm_config,
        }
        return await self._run_remote(
            function_name=self.infra.settings.modal_evaluation_function_name,
            payload=payload,
            gpu=self._resolve_gpu(gpu_class),
            table="evaluations",
            row_id=evaluation_id,
            tenant_id=tenant_id,
            label="evaluation",
            clear_after=False,
        )


def create_gpu_provider(infra, provider_name: str = "local") -> GpuProvider:
    """Factory function to create the configured GPU provider.

    Args:
        infra: InfraContainer with S3, DB, Redis clients
        provider_name: "local" or "modal"
    """
    if provider_name == "modal":
        logger.info("GPU provider: Modal (serverless)")
        return ModalGpuProvider(infra)
    elif provider_name == "local":
        logger.info("GPU provider: Local (worker GPU)")
        return LocalGpuProvider(infra)
    else:
        raise ValueError(f"Unknown GPU provider: {provider_name}. Valid options: local, modal")
