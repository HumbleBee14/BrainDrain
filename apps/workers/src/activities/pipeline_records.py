"""Row provisioning for the one-click pipeline, and the teacher pass's own row.

The per-stage API routes create their training_jobs / evaluations rows in Rust
before starting a workflow. FullPipelineWorkflow has no such route per stage,
so these activities create the rows mid-pipeline — after the dataset exists —
giving the training and evaluation activities real ids to claim and update.

The teacher-extraction lifecycle activity also carries extraction's billing, for
the same reason `train_model` carries training's: the charge and the state
transition it describes have to commit together or not at all, and only the
worker knows what the GPU actually cost.
"""

import json
import logging
import uuid
from dataclasses import dataclass, field

from temporalio import activity

from src.constants import GPU_DEFAULT_HOURLY_RATE, GPU_HOURLY_RATES
from src.infra import InfraContainer

logger = logging.getLogger("platform.pipeline_records")

# The two runtimes the scoring pass measures: pulling and loading tens of
# gigabytes of weights, then scoring. Both are GPU time we pay for.
_MEASURED_RUNTIME_KEYS = ("teacher_load_seconds", "scoring_seconds")


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
    # Distill mode: teacher block recorded on the job row. Any api_key inside
    # is enc:v1-encrypted; plaintext never reaches this payload.
    teacher_config: dict | None = None


@dataclass
class CreateEvaluationInput:
    tenant_id: str
    model_id: str


@dataclass
class MarkDatasetFailedInput:
    tenant_id: str
    dataset_id: str
    error: str


class TeacherExtractionStatus:
    """Lifecycle of the teacher scoring pass, tracked apart from the training run.

    A job that never asks for a fidelity upgrade leaves the column NULL. A run
    abandoned mid-scoring keeps RUNNING on purpose: that is what tells an operator
    (and the orphan sweep) that the GPU still owed money was the teacher's, not
    the student's.
    """

    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"


@dataclass
class SetTeacherExtractionStatusInput:
    tenant_id: str
    training_job_id: str
    status: str
    # The scoring pass's own metrics, present only on a terminal transition that
    # had a result to report. Appended with a default so a payload serialized
    # before it existed still binds; `None` means "runtime unknown", which the
    # charge below is careful not to read as "no runtime".
    metrics: dict | None = None


def extraction_billing_event_id(job_id: str) -> uuid.UUID:
    """Stable ledger id for one job's extraction pass.

    Derived from the job rather than generated, so the reservation written when
    scoring starts and the finalization written when it ends address the same
    row — and a Temporal retry of either addresses it again instead of charging
    the tenant twice.
    """
    return uuid.uuid5(uuid.NAMESPACE_URL, f"extraction-billing:{job_id}")


def measured_extraction_seconds(metrics: dict) -> float:
    """GPU seconds the scoring pass reports having used.

    A pass that found a previous run's artifacts already committed reports
    neither runtime and costs nothing, which is the truth: no GPU was started.
    """
    return sum(
        float(metrics[key])
        for key in _MEASURED_RUNTIME_KEYS
        if isinstance(metrics.get(key), int | float) and not isinstance(metrics[key], bool)
    )


def extraction_charge(
    *,
    metrics: dict | None,
    elapsed_seconds: float | None,
    gpu_class: str | None,
    estimate: tuple[int, float],
    min_billable_seconds: int,
) -> tuple[int, float]:
    """GPU seconds and dollars to bill for one finished extraction pass.

    Three sources of truth, in descending order of how much we trust them, and
    the same order training uses: the run's own measured runtimes; failing that
    the age of its reservation, which brackets the GPU session it held and is
    voided below the threshold a failed training run is voided at; failing that
    the admission estimate, which is deliberately pessimistic and is what the
    tenant already agreed to.
    """
    rate = GPU_HOURLY_RATES.get((gpu_class or "").lower(), GPU_DEFAULT_HOURLY_RATE)

    if metrics is not None:
        seconds = measured_extraction_seconds(metrics)
        return int(round(seconds)), _dollars(seconds, rate)
    if elapsed_seconds is None:
        return estimate
    if elapsed_seconds < min_billable_seconds:
        return int(round(elapsed_seconds)), 0.0
    return int(round(elapsed_seconds)), _dollars(elapsed_seconds, rate)


def _dollars(gpu_seconds: float, hourly_rate: float) -> float:
    return round(gpu_seconds / 3600.0 * hourly_rate, 2)


def _estimated_charge(est_gpu_hours: float | None, est_cost_usd: float | None) -> tuple[int, float]:
    return int(round((est_gpu_hours or 0.0) * 3600)), round(est_cost_usd or 0.0, 2)


async def _reserve_extraction_charge(
    conn,
    *,
    tenant_id: str,
    job_id: str,
    gpu_class: str | None,
    estimate: tuple[int, float],
) -> None:
    """Commit a conservative charge for the GPU pass that is about to start.

    Written before the worker hands control to the teacher, because after that
    point a crash, a termination or a cancellation can end the run without ever
    coming back — and a GPU that ran unbilled is a GPU the spend cap never sees.
    The row stays out of the ledger while `extraction_pending` is set: the API's
    relay either finalizes it with actuals or, once the run cannot plausibly
    still be alive, reaps it at this estimate.
    """
    gpu_seconds, cost_usd = estimate
    await conn.execute(
        """INSERT INTO billing_outbox
            (id, tenant_id, operation, resource_id, tokens_in, tokens_out,
             gpu_seconds, cost_usd, metadata)
        VALUES ($1, $2::uuid, 'extraction', $3::uuid, 0, 0, $4, $5, $6::jsonb)
        ON CONFLICT (id) DO NOTHING""",
        extraction_billing_event_id(job_id),
        tenant_id,
        job_id,
        gpu_seconds,
        cost_usd,
        json.dumps(
            {
                "extraction_pending": True,
                "extraction_reaped": False,
                "status": TeacherExtractionStatus.RUNNING,
                "gpu_class": gpu_class,
                "est_cost_usd": cost_usd,
            }
        ),
    )


async def _finalize_extraction_charge(
    conn,
    *,
    tenant_id: str,
    job_id: str,
    status: str,
    gpu_class: str | None,
    gpu_seconds: int,
    cost_usd: float,
) -> float | None:
    """Replace the reservation's estimate with what the GPU cost; return the bill.

    An upsert rather than an update so the charge survives every ordering this
    can run in: it corrects a reservation still awaiting delivery, and it writes
    one outright for a run whose reservation was never committed. It declines to
    touch a row the relay has already delivered — that charge is in the ledger at
    the estimate and correcting it here would only diverge the two.
    """
    row_id = extraction_billing_event_id(job_id)
    metadata = json.dumps({"extraction_pending": False, "status": status, "gpu_class": gpu_class})
    billed = await conn.fetchval(
        """INSERT INTO billing_outbox
            (id, tenant_id, operation, resource_id, tokens_in, tokens_out,
             gpu_seconds, cost_usd, metadata)
        VALUES ($1, $2::uuid, 'extraction', $3::uuid, 0, 0, $4, $5, $6::jsonb)
        ON CONFLICT (id) DO UPDATE
            SET gpu_seconds = EXCLUDED.gpu_seconds,
                cost_usd = EXCLUDED.cost_usd,
                metadata = billing_outbox.metadata || EXCLUDED.metadata
            WHERE billing_outbox.delivered_at IS NULL
              AND billing_outbox.tenant_id = EXCLUDED.tenant_id
        RETURNING cost_usd""",
        row_id,
        tenant_id,
        job_id,
        gpu_seconds,
        cost_usd,
        metadata,
    )
    if billed is None:
        billed = await conn.fetchval(
            "SELECT cost_usd FROM billing_outbox WHERE id = $1 AND tenant_id = $2::uuid",
            row_id,
            tenant_id,
        )
        logger.warning(
            "Extraction charge for job %s was already delivered; keeping the billed $%s",
            job_id,
            billed,
        )
    return None if billed is None else float(billed)


class CreateTrainingJobActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="create_training_job")
    async def run(self, input: CreateTrainingJobInput) -> str:
        """Insert a pending training_jobs row and return its id."""
        job_id = await self.infra.db.fetchval(
            """INSERT INTO training_jobs
                (tenant_id, project_id, dataset_id, base_model, method, mode,
                 hyperparams, gpu_class, teacher_config)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb)
            RETURNING id""",
            input.tenant_id,
            input.project_id,
            input.dataset_id,
            input.base_model,
            input.method,
            input.mode,
            json.dumps(input.hyperparams),
            input.gpu_class,
            json.dumps(input.teacher_config) if input.teacher_config is not None else None,
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


class SetTeacherExtractionStatusActivity:
    """Where the teacher scoring pass got to, and what its GPU time cost.

    One activity for both because they are one fact: the transition into RUNNING
    is what commits the reservation, and the transition out of it is what replaces
    that reservation with the measured bill. Splitting them would open a window in
    which the job says one thing and the ledger another.
    """

    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="set_teacher_extraction_status")
    async def run(self, input: SetTeacherExtractionStatusInput) -> None:
        async with self.infra.db.acquire() as conn:
            async with conn.transaction():
                context = await conn.fetchrow(
                    """SELECT (t.teacher_config->'extraction'->>'est_cost_usd')::float8
                                  AS est_cost_usd,
                              (t.teacher_config->'extraction'->>'est_gpu_hours')::float8
                                  AS est_gpu_hours,
                               t.teacher_config->'extraction'->>'gpu_class' AS gpu_class,
                              (SELECT EXTRACT(EPOCH FROM (NOW() - created_at))::float8
                                 FROM billing_outbox
                                WHERE id = $3 AND tenant_id = $2::uuid) AS reserved_seconds_ago
                         FROM training_jobs t
                        WHERE t.id = $1::uuid AND t.tenant_id = $2::uuid""",
                    input.training_job_id,
                    input.tenant_id,
                    extraction_billing_event_id(input.training_job_id),
                )
                if context is None:
                    logger.warning(
                        "No training job %s for tenant %s; extraction status not recorded",
                        input.training_job_id,
                        input.tenant_id,
                    )
                    return

                gpu_class = context["gpu_class"]
                estimate = _estimated_charge(context["est_gpu_hours"], context["est_cost_usd"])

                if input.status == TeacherExtractionStatus.RUNNING:
                    await _reserve_extraction_charge(
                        conn,
                        tenant_id=input.tenant_id,
                        job_id=input.training_job_id,
                        gpu_class=gpu_class,
                        estimate=estimate,
                    )
                    cost = None
                else:
                    gpu_seconds, cost_usd = extraction_charge(
                        metrics=input.metrics,
                        elapsed_seconds=context["reserved_seconds_ago"],
                        gpu_class=gpu_class,
                        estimate=estimate,
                        min_billable_seconds=getattr(
                            self.infra.settings, "min_billable_seconds", 300
                        ),
                    )
                    cost = await _finalize_extraction_charge(
                        conn,
                        tenant_id=input.tenant_id,
                        job_id=input.training_job_id,
                        status=input.status,
                        gpu_class=gpu_class,
                        gpu_seconds=gpu_seconds,
                        cost_usd=cost_usd,
                    )

                await conn.execute(
                    """UPDATE training_jobs
                    SET teacher_extraction_status = $3,
                        teacher_extraction_cost = COALESCE($4, teacher_extraction_cost)
                    WHERE id = $1::uuid AND tenant_id = $2::uuid""",
                    input.training_job_id,
                    input.tenant_id,
                    input.status,
                    cost,
                )

        logger.info(
            "Teacher extraction for job %s is %s (billed %s)",
            input.training_job_id,
            input.status,
            f"${cost:.2f}" if cost is not None else "nothing yet",
        )


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
