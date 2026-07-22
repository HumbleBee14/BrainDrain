"""GPU compute provider abstraction for training jobs.

Provides a GpuProvider protocol with pluggable implementations:
  - LocalGpuProvider: Runs training on the worker's local GPU (dev/testing)
  - ModalGpuProvider: Sends training to Modal serverless GPUs (production)

The provider is selected by the APP_GPU_PROVIDER config:
  - "local" (default): Use the worker's own GPU
  - "modal": Provision ephemeral GPU on Modal

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

    Implementations handle the full training lifecycle:
    download data → load model → train → upload adapter.
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
        """Execute a training job and return result dict.

        Returns:
            dict with keys: adapter_path, adapter_size_bytes, metrics
        """
        ...


class LocalGpuProvider:
    """Run training on the local worker's GPU.

    This is the default provider for development and single-machine setups.
    The worker process must have access to a CUDA-capable GPU.
    Training runs in-process — same behavior as before the provider abstraction.
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


class ModalGpuProvider:
    """Run training on Modal serverless GPUs via a pre-deployed app.

    Invokes the deployed `train` function (see apps/workers/modal_app.py) with
    spawn/poll so the worker event loop never blocks. Persists the Modal
    FunctionCall id to training_jobs.modal_call_id BEFORE polling, so an
    activity retry / worker restart recovers the in-flight job instead of
    launching (and paying for) a duplicate GPU run.

    The remote function is pure-compute; this provider (worker-side) owns the
    reservation DB writes.
    """

    def __init__(self, infra):
        self.infra = infra
        self._validate_modal_available()

    def _validate_modal_available(self):
        try:
            import modal  # noqa: F401
        except ImportError as e:
            raise RuntimeError(
                "Modal is not installed. Install the gpu-cloud extra: "
                "uv sync --extra gpu-cloud"
            ) from e

    def _resolve_gpu(self, gpu_class: str | None) -> str:
        from src.constants import MODAL_DEFAULT_GPU, MODAL_GPU_MAP

        return MODAL_GPU_MAP.get(gpu_class or "", MODAL_DEFAULT_GPU)

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
        import asyncio

        import modal
        from temporalio import activity

        settings = self.infra.settings
        gpu = self._resolve_gpu(gpu_class)

        # 1. Recover an in-flight call if one was already reserved for this job.
        existing = await self.infra.db.fetchval(
            "SELECT modal_call_id FROM training_jobs WHERE id = $1 AND tenant_id = $2",
            training_job_id,
            tenant_id,
        )

        if existing:
            logger.info("Recovering Modal call %s for job %s", existing, training_job_id[:8])
            fc = modal.FunctionCall.from_id(existing)
        else:
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
            fn = modal.Function.from_name(settings.modal_app_name, settings.modal_function_name)
            logger.info(
                "Spawning Modal training (job=%s, gpu=%s, model=%s)",
                training_job_id[:8],
                gpu,
                base_model,
            )
            fc = await fn.options(gpu=gpu).spawn.aio(payload)

            # 2. Reservation: persist BEFORE polling so a crash reconnects, no respawn.
            await self.infra.db.execute(
                "UPDATE training_jobs SET modal_call_id = $1 WHERE id = $2 AND tenant_id = $3",
                fc.object_id,
                training_job_id,
                tenant_id,
            )

        # 3. Non-blocking poll until the remote run completes.
        while True:
            try:
                result = await fc.get.aio(timeout=0)
                break
            except TimeoutError:
                activity.heartbeat()
                await asyncio.sleep(settings.modal_poll_interval_secs)

        logger.info("Modal training complete (job=%s)", training_job_id[:8])
        return result


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
