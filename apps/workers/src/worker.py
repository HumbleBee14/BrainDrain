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

from temporalio.client import Client
from temporalio.worker import Worker

from src.config import WorkerSettings
from src.infra import init_container
from src.workflows.evaluate import EvaluateWorkflow
from src.workflows.full_pipeline import FullPipelineWorkflow
from src.workflows.ingest import IngestWorkflow
from src.workflows.refine import RefineWorkflow
from src.workflows.train import TrainWorkflow
from src.workflows.train_aligned import TrainAlignedWorkflow
from src.workflows.train_iterative import TrainIterativeWorkflow
from src.workflows.train_reasoning import TrainReasoningWorkflow


async def main() -> None:
    settings = WorkerSettings()

    # Set HuggingFace env vars before any ML imports
    if settings.hf_token:
        os.environ["HF_TOKEN"] = settings.hf_token
    os.environ["HF_HOME"] = settings.model_cache_dir

    logging.basicConfig(level=getattr(logging, settings.log_level))
    logger = logging.getLogger("platform.worker")

    # Initialize infrastructure container (S3, DB, Redis)
    logger.info("Initializing infrastructure...")
    infra = await init_container(settings)

    logger.info("Connecting to Temporal at %s", settings.temporal_address)
    client = await Client.connect(
        settings.temporal_address,
        namespace=settings.temporal_namespace,
    )

    # Import and instantiate activity classes with injected infrastructure
    from src.activities.build_dataset import BuildDatasetActivity
    from src.activities.chunk_text import ChunkTextActivity
    from src.activities.generate_pairs import GeneratePairsActivity
    from src.activities.parse_document import ParseDocumentActivity
    from src.activities.run_evaluation import RunEvaluationActivity
    from src.activities.stubs import DeployModelActivity, GetDocumentInfoActivity
    from src.activities.train_model import (
        EvaluateHoldoutActivity,
        StartTrainingActivity,
        TrainSftRoundActivity,
    )

    # CPU-bound activities (parsing, data generation, dataset building)
    cpu_activities = [
        ParseDocumentActivity(infra),
        GeneratePairsActivity(infra),
        ChunkTextActivity(infra),
        BuildDatasetActivity(infra),
        GetDocumentInfoActivity(infra),
    ]

    # GPU-bound activities (training, evaluation)
    gpu_activities = [
        StartTrainingActivity(infra),
        TrainSftRoundActivity(infra),
        EvaluateHoldoutActivity(infra),
        RunEvaluationActivity(infra),
        DeployModelActivity(infra),
    ]

    # All workflows (registered on every worker mode)
    all_workflows = [
        IngestWorkflow,
        RefineWorkflow,
        TrainWorkflow,
        TrainIterativeWorkflow,
        TrainAlignedWorkflow,
        TrainReasoningWorkflow,
        EvaluateWorkflow,
        FullPipelineWorkflow,
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
