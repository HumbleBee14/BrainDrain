"""Row provisioning for the one-click pipeline.

The per-stage API routes create their training_jobs / evaluations rows in Rust
before starting a workflow. FullPipelineWorkflow has no such route per stage,
so these activities create the rows mid-pipeline — after the dataset exists —
giving the training and evaluation activities real ids to claim and update.
"""

import json
import logging
from dataclasses import dataclass, field

from temporalio import activity

from src.infra import InfraContainer

logger = logging.getLogger("platform.pipeline_records")


@dataclass
class CreateTrainingJobInput:
    tenant_id: str
    project_id: str
    dataset_id: str
    base_model: str
    method: str
    mode: str
    hyperparams: dict = field(default_factory=dict)
    gpu_class: str | None = None


@dataclass
class CreateEvaluationInput:
    tenant_id: str
    model_id: str


@dataclass
class MarkDatasetFailedInput:
    tenant_id: str
    dataset_id: str
    error: str


class CreateTrainingJobActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="create_training_job")
    async def run(self, input: CreateTrainingJobInput) -> str:
        """Insert a pending training_jobs row and return its id."""
        job_id = await self.infra.db.fetchval(
            """INSERT INTO training_jobs
                (tenant_id, project_id, dataset_id, base_model, method, mode,
                 hyperparams, gpu_class)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id""",
            input.tenant_id,
            input.project_id,
            input.dataset_id,
            input.base_model,
            input.method,
            input.mode,
            json.dumps(input.hyperparams),
            input.gpu_class,
        )
        logger.info("Pipeline created training job %s", job_id)
        return str(job_id)


class CreateEvaluationActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="create_evaluation")
    async def run(self, input: CreateEvaluationInput) -> str:
        """Insert an evaluations row (same shape the per-stage route creates)."""
        eval_id = await self.infra.db.fetchval(
            """INSERT INTO evaluations (tenant_id, model_id, status, started_at)
            VALUES ($1, $2, 'running', NOW())
            RETURNING id""",
            input.tenant_id,
            input.model_id,
        )
        logger.info("Pipeline created evaluation %s for model %s", eval_id, input.model_id)
        return str(eval_id)


class MarkDatasetFailedActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="mark_dataset_failed")
    async def run(self, input: MarkDatasetFailedInput) -> None:
        """Record why a reserved dataset row never produced pairs."""
        await self.infra.db.execute(
            """UPDATE datasets
            SET status = 'failed', error = $3, updated_at = NOW()
            WHERE id = $1::uuid AND tenant_id = $2::uuid""",
            input.dataset_id,
            input.tenant_id,
            input.error[:2000],
        )
        logger.warning("Dataset %s marked failed: %s", input.dataset_id, input.error[:200])
