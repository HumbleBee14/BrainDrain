"""Temporal worker entrypoint.

Starts a worker that listens on the ML pipeline task queue
and executes registered workflows and activities.

Supports three worker modes via APP_WORKER_MODE:
  - "all": Register all activities on a single queue (dev mode)
  - "main": CPU activities only on ml-pipeline-main queue
  - "gpu": GPU activities only on ml-pipeline-gpu queue
"""

import asyncio
import logging
import os
from collections.abc import Sequence

from temporalio.client import Client, Interceptor
from temporalio.worker import Worker

from src.config import WorkerSettings
from src.infra import InfraContainer, init_container
from src.workflows.datagen import (
    GenerateDatasetWorkflow,
    GenerateFacetsWorkflow,
    GeneratePreviewWorkflow,
    RefineGuidanceWorkflow,
)
from src.workflows.evaluate import EvaluateWorkflow
from src.workflows.export import ExportWorkflow
from src.workflows.full_pipeline import FullPipelineWorkflow
from src.workflows.ingest import IngestWorkflow
from src.workflows.refine import RefineWorkflow
from src.workflows.train import TrainWorkflow
from src.workflows.train_aligned import TrainAlignedWorkflow
from src.workflows.train_iterative import TrainIterativeWorkflow
from src.workflows.train_reasoning import TrainReasoningWorkflow


def setup_logging(settings: WorkerSettings) -> None:
    """Configure structured logging for all platform loggers.

    Uses JSON format by default for production (machine-parseable, Loki-friendly).
    Set APP_LOG_FORMAT=text for human-readable output in development.
    """
    root = logging.getLogger()
    root.setLevel(getattr(logging, settings.log_level, logging.INFO))

    # Remove any existing handlers (prevent duplicate output)
    root.handlers.clear()

    handler = logging.StreamHandler()

    if settings.log_format.lower() == "text":
        formatter = logging.Formatter(
            "%(asctime)s %(levelname)-8s [%(name)s] %(message)s",
            datefmt="%Y-%m-%dT%H:%M:%S",
        )
    else:
        from pythonjsonlogger.json import JsonFormatter

        formatter = JsonFormatter(
            fmt="%(asctime)s %(levelname)s %(name)s %(message)s",
            rename_fields={"asctime": "timestamp", "levelname": "level", "name": "logger"},
            datefmt="%Y-%m-%dT%H:%M:%S%z",
        )

    handler.setFormatter(formatter)
    root.addHandler(handler)


def init_otel(settings: WorkerSettings) -> Sequence[Interceptor]:
    """Initialize OpenTelemetry tracing if enabled.

    Returns a list of Temporal interceptors to attach to the client.
    Best-effort: if collector is unreachable, traces are dropped silently.
    Swap OTEL for another vendor by changing only this function.
    """
    if not settings.otel_enabled:
        return []

    try:
        from opentelemetry import trace
        from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import OTLPSpanExporter
        from opentelemetry.sdk.resources import Resource
        from opentelemetry.sdk.trace import TracerProvider
        from opentelemetry.sdk.trace.export import BatchSpanProcessor
        from temporalio.contrib.opentelemetry import TracingInterceptor

        resource = Resource.create(
            {
                "service.name": "platform-worker",
                "deployment.environment": "development",
            }
        )
        exporter = OTLPSpanExporter(endpoint=settings.otel_endpoint, insecure=True)
        provider = TracerProvider(resource=resource)
        provider.add_span_processor(BatchSpanProcessor(exporter))
        trace.set_tracer_provider(provider)

        # Inject trace_id / span_id into all log records
        from opentelemetry.instrumentation.logging import LoggingInstrumentor

        LoggingInstrumentor().instrument(set_logging_format=False)

        logging.getLogger("platform.worker").info(
            "OpenTelemetry export enabled → %s", settings.otel_endpoint
        )
        return [TracingInterceptor()]
    except Exception:
        logging.getLogger("platform.worker").warning(
            "Failed to initialize OpenTelemetry, continuing without tracing",
            exc_info=True,
        )
        return []


def build_activity_lists(infra: InfraContainer, gpu_provider: object) -> tuple[list, list]:
    """Build the (cpu, gpu) activity callables registered with the Temporal worker.

    Temporal registers activities by their ``@activity.defn``-decorated callable —
    the bound ``run`` method, not the holder instance. Passing the instance raises
    ``TypeError: Activity <unknown> missing attributes`` at ``Worker(...)`` construction,
    so this is exercised by a startup smoke test.

    ML-heavy activity modules are imported lazily so importing this module (and running
    fast unit tests) does not pull in torch/transformers.
    """
    from src.activities.build_dataset import BuildDatasetActivity
    from src.activities.chunk_text import ChunkTextActivity
    from src.activities.datagen_activities import (
        GenerateFacetsActivity,
        GeneratePreviewActivity,
        RefineGuidanceActivity,
        UpdateDataGuideActivity,
    )
    from src.activities.export_gguf import ExportGgufActivity
    from src.activities.generate_pairs import GeneratePairsActivity
    from src.activities.parse_document import ParseDocumentActivity
    from src.activities.run_evaluation import RunEvaluationActivity
    from src.activities.stubs import DeployModelActivity, GetDocumentInfoActivity
    from src.activities.train_model import (
        EvaluateHoldoutActivity,
        FinalizeIterativeTrainingActivity,
        StartTrainingActivity,
        TrainSftRoundActivity,
    )

    # CPU-bound activities (parsing, data generation, dataset building)
    cpu_activities = [
        ParseDocumentActivity(infra).run,
        GeneratePairsActivity(infra).run,
        ChunkTextActivity(infra).run,
        BuildDatasetActivity(infra).run,
        GetDocumentInfoActivity(infra).run,
        GenerateFacetsActivity(infra).run,
        GeneratePreviewActivity(infra).run,
        RefineGuidanceActivity(infra).run,
        UpdateDataGuideActivity(infra).run,
    ]

    # GPU-bound activities (training, evaluation)
    gpu_activities = [
        StartTrainingActivity(infra, gpu_provider=gpu_provider).run,
        TrainSftRoundActivity(infra, gpu_provider=gpu_provider).run,
        EvaluateHoldoutActivity(infra, gpu_provider=gpu_provider).run,
        FinalizeIterativeTrainingActivity(infra).run,
        RunEvaluationActivity(infra, gpu_provider=gpu_provider).run,
        DeployModelActivity(infra).run,
        ExportGgufActivity(infra).run,
    ]

    return cpu_activities, gpu_activities


async def main() -> None:
    settings = WorkerSettings()

    # Set HuggingFace env vars before any ML imports
    if settings.hf_token:
        os.environ["HF_TOKEN"] = settings.hf_token
    os.environ["HF_HOME"] = settings.model_cache_dir

    setup_logging(settings)
    logger = logging.getLogger("platform.worker")

    # Initialize OpenTelemetry (best-effort, never blocks startup)
    interceptors = init_otel(settings)

    # Initialize infrastructure container (S3, DB, Redis)
    logger.info("Initializing infrastructure...")
    infra = await init_container(settings)

    logger.info("Connecting to Temporal at %s", settings.temporal_address)
    client = await Client.connect(
        settings.temporal_address,
        namespace=settings.temporal_namespace,
        interceptors=interceptors,
    )

    # Initialize GPU provider (local by default, Modal for serverless GPUs)
    from src.gpu_provider import create_gpu_provider

    gpu_provider = create_gpu_provider(infra, settings.gpu_provider)

    cpu_activities, gpu_activities = build_activity_lists(infra, gpu_provider)

    # All workflows (registered on every worker mode)
    all_workflows = [
        IngestWorkflow,
        RefineWorkflow,
        TrainWorkflow,
        TrainIterativeWorkflow,
        TrainAlignedWorkflow,
        TrainReasoningWorkflow,
        EvaluateWorkflow,
        ExportWorkflow,
        FullPipelineWorkflow,
        GenerateFacetsWorkflow,
        GeneratePreviewWorkflow,
        RefineGuidanceWorkflow,
        GenerateDatasetWorkflow,
    ]

    mode = settings.worker_mode

    if mode == "main":
        task_queue = "ml-pipeline-main"
        activities = cpu_activities
        logger.info("Worker mode: main (CPU only)")
    elif mode == "gpu":
        task_queue = "ml-pipeline-gpu"
        activities = gpu_activities
        logger.info("Worker mode: gpu (GPU only)")
    else:
        task_queue = settings.temporal_task_queue
        activities = cpu_activities + gpu_activities
        logger.info("Worker mode: all (dev mode)")

    logger.info("Starting worker on queue: %s", task_queue)
    worker = Worker(
        client,
        task_queue=task_queue,
        workflows=all_workflows,
        activities=activities,
    )

    logger.info("Worker started. Waiting for tasks...")
    try:
        await worker.run()
    finally:
        from src.infra import close_container

        await close_container()


if __name__ == "__main__":
    asyncio.run(main())
