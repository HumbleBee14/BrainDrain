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
    """Run training on Modal serverless GPUs.

    Provisions an ephemeral GPU container on Modal, executes the training
    job, and returns results. The container auto-terminates after training.

    Requires:
      - modal package installed (pip install modal)
      - MODAL_TOKEN_ID and MODAL_TOKEN_SECRET env vars set
      - APP_GPU_PROVIDER=modal in worker config

    GPU selection:
      - gpu_class maps to Modal GPU types: "A10G", "A100", "H100"
      - Default: "A10G" (cost-effective for 7B-13B LoRA fine-tuning)
    """

    # Map platform gpu_class to Modal GPU specifiers
    GPU_MAP = {
        "A10G": "a10g",
        "A100": "a100",
        "A100-80GB": "a100-80gb",
        "H100": "h100",
    }

    DEFAULT_GPU = "a10g"

    def __init__(self, infra):
        self.infra = infra
        self._validate_modal_available()

    def _validate_modal_available(self):
        try:
            import modal  # noqa: F401
        except ImportError:
            raise RuntimeError(
                "Modal is not installed. Install with: pip install modal\n"
                "Or add 'modal' to pyproject.toml optional dependencies."
            )

    def _resolve_gpu(self, gpu_class: str | None) -> str:
        if gpu_class and gpu_class in self.GPU_MAP:
            return self.GPU_MAP[gpu_class]
        return self.DEFAULT_GPU

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
    ) -> dict:
        import modal

        gpu_spec = self._resolve_gpu(gpu_class)

        logger.info(
            "Provisioning Modal GPU (job=%s, gpu=%s, model=%s)",
            training_job_id[:8],
            gpu_spec,
            base_model,
        )

        # Build the Modal image with all ML dependencies
        image = modal.Image.debian_slim(python_version="3.11").pip_install(
            "unsloth>=2025.12",
            "transformers>=4.51.0,<5.0.0",
            "datasets>=3.2.0",
            "trl>=0.16.0",
            "peft>=0.14.0",
            "accelerate>=1.2.0",
            "bitsandbytes>=0.45.0",
            "pynvml>=12.0.0",
            "boto3>=1.35.0",
            "asyncpg>=0.29.0",
            "redis>=5.0.0",
            "httpx>=0.27.0",
            "temporalio>=1.9.0",
            "pydantic>=2.10.0",
            "pydantic-settings>=2.7.0",
        )

        app = modal.App(f"training-{training_job_id[:8]}")

        # Pass infra config as secrets (not the clients themselves)
        settings = self.infra.settings

        @app.function(
            image=image,
            gpu=gpu_spec,
            timeout=6 * 3600,  # 6 hours max
            secrets=[
                modal.Secret.from_dict(
                    {
                        "APP_DATABASE_URL": settings.database_url,
                        "APP_REDIS_URL": settings.redis_url,
                        "APP_S3_ENDPOINT": settings.s3_endpoint,
                        "APP_S3_ACCESS_KEY": settings.s3_access_key,
                        "APP_S3_SECRET_KEY": settings.s3_secret_key,
                        "APP_S3_BUCKET": settings.s3_bucket,
                        "APP_S3_REGION": settings.s3_region,
                        "APP_LLM_API_BASE_URL": settings.llm_api_base_url,
                        "APP_LLM_API_KEY": settings.llm_api_key,
                        "APP_LLM_MODEL": settings.llm_model,
                        "APP_HF_TOKEN": settings.hf_token or "",
                        "HF_HOME": "/tmp/hf_cache",
                    }
                ),
            ],
        )
        async def remote_train(
            t_tenant_id: str,
            t_job_id: str,
            t_dataset_path: str,
            t_base_model: str,
            t_method: str,
            t_mode: str,
            t_hyperparams: dict,
            t_gpu_class: str | None,
        ) -> dict:
            """This function runs on the Modal GPU container."""
            import os

            os.environ.setdefault("HF_HOME", "/tmp/hf_cache")

            # Initialize infrastructure inside the Modal container
            from src.config import WorkerSettings
            from src.infra import init_container

            remote_settings = WorkerSettings()
            remote_infra = await init_container(remote_settings)

            from src.activities.stubs import StartTrainingInput
            from src.activities.train_model import _run_training

            input_data = StartTrainingInput(
                tenant_id=t_tenant_id,
                training_job_id=t_job_id,
                dataset_path=t_dataset_path,
                base_model=t_base_model,
                method=t_method,
                mode=t_mode,
                hyperparams=t_hyperparams,
                gpu_class=t_gpu_class,
            )

            result = await _run_training(input_data, remote_infra)

            from src.infra import close_container

            await close_container()

            return {
                "adapter_path": result.adapter_path,
                "adapter_size_bytes": result.adapter_size_bytes,
                "metrics": result.metrics,
            }

        # Execute on Modal — this blocks until training completes
        with app.run():
            result = remote_train.remote(
                tenant_id,
                training_job_id,
                dataset_path,
                base_model,
                method,
                mode,
                hyperparams,
                gpu_class,
            )

        logger.info(
            "Modal training complete (job=%s, adapter=%s)",
            training_job_id[:8],
            result.get("adapter_path"),
        )
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
