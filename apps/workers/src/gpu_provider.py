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

    async def run_export_gguf(
        self,
        *,
        tenant_id: str,
        model_id: str,
        export_id: str,
        adapter_path: str,
        base_model: str,
        quant_type: str,
    ) -> dict:
        """Merge the adapter and produce a quantized GGUF in object storage.

        Returns: dict with keys storage_path, file_size_bytes.
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


def _extract_call_id(stored: str | None) -> str | None:
    """Return the bare Modal FunctionCall id from a stored reservation value.

    Reservations are stored tagged ``"<function_name>:<call_id>"``; the earliest
    single-shot-training release stored a bare id. Returns ``None`` for an empty
    value.
    """
    if not stored:
        return None
    _tag, sep, call_id = stored.partition(":")
    return call_id if sep else stored


async def _cancel_function_call(fc) -> None:
    """Cancel a Modal ``FunctionCall``, tolerating sync/async cancel APIs.

    Modal exposes async variants as ``method.aio``; fall back to a plain call
    (awaiting it if it happens to return a coroutine) so this works across SDK
    versions.
    """
    import inspect

    cancel = getattr(fc, "cancel", None)
    if cancel is None:
        return
    aio = getattr(cancel, "aio", None)
    if aio is not None:
        await aio()
        return
    result = cancel()
    if inspect.isawaitable(result):
        await result


# Terminal, abandoned job states whose lingering Modal reservation means a GPU
# call is still running (and billing) with nothing left to consume its result.
# A user cancel terminates the workflow -> status 'cancelled'; the reaper marks
# a stuck job 'failed'. Neither clears modal_call_id, and neither stops the
# remote call. Completed rows are excluded — their call has already finished.
_ORPHAN_SWEEP_QUERY = """
    SELECT id, tenant_id, modal_call_id, 'training_jobs' AS tbl
    FROM training_jobs
    WHERE status IN ('cancelled', 'failed') AND modal_call_id IS NOT NULL
    UNION ALL
    SELECT id, tenant_id, modal_call_id, 'evaluations' AS tbl
    FROM evaluations
    WHERE status = 'failed' AND modal_call_id IS NOT NULL
"""

_ORPHAN_CLEAR_SQL = {
    "training_jobs": (
        "UPDATE training_jobs SET modal_call_id = NULL WHERE id = $1 AND tenant_id = $2"
    ),
    "evaluations": ("UPDATE evaluations SET modal_call_id = NULL WHERE id = $1 AND tenant_id = $2"),
}


async def cancel_orphaned_gpu_calls(infra) -> int:
    """Cancel Modal GPU calls left running by cancelled or reaped jobs.

    A user cancel terminates the Temporal workflow and the reaper marks a stuck
    job failed — but neither stops the remote Modal ``FunctionCall``, so the GPU
    keeps running and billing until it finishes on its own. This reconciliation
    finds rows in a terminal-but-abandoned state that still carry a
    ``modal_call_id``, cancels the call, and clears the reservation.

    Safe by construction: only rows whose status is already terminal are
    touched, so an actively-running job is never cancelled, and this never
    depends on Temporal cancellation delivery. Cancelling an already-finished
    call is a harmless no-op. Idempotent — clearing the reservation stops the
    row from being reprocessed on the next sweep.

    Returns the number of Modal calls cancelled.
    """
    import modal

    db = infra.db
    rows = await db.fetch(_ORPHAN_SWEEP_QUERY)
    cancelled = 0
    for row in rows:
        call_id = _extract_call_id(row["modal_call_id"])
        if call_id:
            try:
                fc = modal.FunctionCall.from_id(call_id)
                await _cancel_function_call(fc)
                cancelled += 1
                logger.info(
                    "Cancelled orphaned Modal call %s for %s %s",
                    call_id,
                    row["tbl"],
                    str(row["id"])[:8],
                )
            except Exception:
                # Leave the reservation in place so a later sweep retries it.
                logger.warning("Failed to cancel orphaned Modal call %s", call_id, exc_info=True)
                continue
        await db.execute(_ORPHAN_CLEAR_SQL[row["tbl"]], row["id"], row["tenant_id"])
    return cancelled

    async def run_export_gguf(
        self,
        *,
        tenant_id: str,
        model_id: str,
        export_id: str,
        adapter_path: str,
        base_model: str,
        quant_type: str,
    ) -> dict:
        import asyncio

        from src.export_core import run_export_core

        logger.info("Exporting GGUF locally (export=%s, %s)", export_id[:8], quant_type)
        return await asyncio.to_thread(
            run_export_core,
            {
                "tenant_id": tenant_id,
                "model_id": model_id,
                "export_id": export_id,
                "adapter_path": adapter_path,
                "base_model": base_model,
                "quant_type": quant_type,
            },
        )


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
        gpu: str | None,
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
            # gpu=None keeps the function's declared resources (CPU-only work).
            target = fn.with_options(gpu=gpu) if gpu else fn
            fc = await target.spawn.aio(payload)
            # A stale, mismatched reservation (if any) is overwritten here — safe,
            # because at most one remote call is ever in flight per row.
            await db.execute(
                f"UPDATE {table} SET modal_call_id = $1 WHERE id = $2 AND tenant_id = $3",  # noqa: S608
                f"{function_name}:{fc.object_id}",
                row_id,
                tenant_id,
            )

        try:
            while True:
                try:
                    result = await fc.get.aio(timeout=0)
                    break
                except TimeoutError:
                    activity.heartbeat()
                    await asyncio.sleep(settings.modal_poll_interval_secs)
        except asyncio.CancelledError:
            # Temporal delivered a graceful cancellation (the user cancelled the
            # job, or a parent workflow was cancelled). Stop the in-flight Modal
            # GPU call NOW so it stops billing immediately, instead of leaving it
            # running until the periodic orphan sweep catches it minutes later.
            # Best-effort: if the cancel cannot be sent, the orphan-sweep backstop
            # (see cancel_orphaned_gpu_calls) still cancels it on its next cycle.
            logger.info("Activity cancelled; cancelling in-flight Modal call %s", fc.object_id)
            try:
                await _cancel_function_call(fc)
            except Exception:
                logger.warning(
                    "Failed to cancel Modal call on activity cancellation; orphan sweep will retry",
                    exc_info=True,
                )
            raise

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

    async def run_export_gguf(
        self,
        *,
        tenant_id: str,
        model_id: str,
        export_id: str,
        adapter_path: str,
        base_model: str,
        quant_type: str,
    ) -> dict:
        payload = {
            "tenant_id": tenant_id,
            "model_id": model_id,
            "export_id": export_id,
            "adapter_path": adapter_path,
            "base_model": base_model,
            "quant_type": quant_type,
        }
        return await self._run_remote(
            function_name=self.infra.settings.modal_export_function_name,
            payload=payload,
            gpu=None,
            table="model_exports",
            row_id=export_id,
            tenant_id=tenant_id,
            label="gguf-export",
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
