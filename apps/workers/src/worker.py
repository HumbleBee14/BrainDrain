"""Temporal worker entrypoint.

Starts a worker that listens on the ML pipeline task queue
and executes registered workflows and activities.
"""

import asyncio
import logging

from temporalio.client import Client
from temporalio.worker import Worker

from src.activities.stubs import (
    build_dataset,
    deploy_model,
    generate_synthetic_pairs,
    parse_document,
    run_evaluation,
    start_training,
)
from src.config import WorkerSettings
from src.workflows.evaluate import EvaluateWorkflow
from src.workflows.full_pipeline import FullPipelineWorkflow
from src.workflows.ingest import IngestWorkflow
from src.workflows.refine import RefineWorkflow
from src.workflows.train import TrainWorkflow


async def main() -> None:
    settings = WorkerSettings()

    logging.basicConfig(level=getattr(logging, settings.log_level))
    logger = logging.getLogger("platform.worker")

    logger.info("Connecting to Temporal at %s", settings.temporal_address)
    client = await Client.connect(
        settings.temporal_address,
        namespace=settings.temporal_namespace,
    )

    logger.info("Starting worker on queue: %s", settings.temporal_task_queue)
    worker = Worker(
        client,
        task_queue=settings.temporal_task_queue,
        workflows=[
            IngestWorkflow,
            RefineWorkflow,
            TrainWorkflow,
            EvaluateWorkflow,
            FullPipelineWorkflow,
        ],
        activities=[
            parse_document,
            generate_synthetic_pairs,
            build_dataset,
            start_training,
            run_evaluation,
            deploy_model,
        ],
    )

    logger.info("Worker started. Waiting for tasks...")
    await worker.run()


if __name__ == "__main__":
    asyncio.run(main())
