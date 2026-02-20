"""Temporal worker entrypoint.

Starts a worker that listens on the ML pipeline task queue
and executes registered workflows and activities.
"""

import asyncio
import logging

from temporalio.client import Client
from temporalio.worker import Worker

from src.activities.stubs import (
    deploy_model,
    get_document_info,
    run_evaluation,
    start_training,
)
from src.clients import close_clients, init_clients
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

    # Initialize infrastructure clients (S3, DB, Redis)
    logger.info("Initializing infrastructure clients...")
    await init_clients(settings)

    logger.info("Connecting to Temporal at %s", settings.temporal_address)
    client = await Client.connect(
        settings.temporal_address,
        namespace=settings.temporal_namespace,
    )

    # Import real activity implementations
    from src.activities.build_dataset import build_dataset
    from src.activities.chunk_text import chunk_text
    from src.activities.generate_pairs import generate_synthetic_pairs
    from src.activities.parse_document import parse_document

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
            chunk_text,
            build_dataset,
            get_document_info,
            start_training,
            run_evaluation,
            deploy_model,
        ],
    )

    logger.info("Worker started. Waiting for tasks...")
    try:
        await worker.run()
    finally:
        await close_clients()


if __name__ == "__main__":
    asyncio.run(main())
