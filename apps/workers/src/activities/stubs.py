"""Stub activities and shared dataclasses for pipeline stages.

Training has a real implementation in train_model.py.
Evaluation and deployment are Phase 3+.
Parsing, data generation, and dataset building have real implementations
in their own modules (parse_document.py, generate_pairs.py, etc.).
"""

from dataclasses import dataclass

from temporalio import activity

from src import clients

# ── Document info (lightweight DB lookup for workflows) ──


@dataclass
class DocumentInfo:
    document_id: str
    tenant_id: str
    project_id: str
    storage_path: str
    mime_type: str
    status: str


@activity.defn
async def get_document_info(document_id: str) -> DocumentInfo:
    """Fetch document metadata from DB. Used by workflows before calling parse."""
    db = await clients.get_db()
    row = await db.fetchrow(
        "SELECT id, tenant_id, project_id, storage_path, mime_type, status "
        "FROM documents WHERE id = $1",
        document_id,
    )
    if row is None:
        raise ValueError(f"Document not found: {document_id}")
    return DocumentInfo(
        document_id=str(row["id"]),
        tenant_id=str(row["tenant_id"]),
        project_id=str(row["project_id"]),
        storage_path=row["storage_path"],
        mime_type=row["mime_type"],
        status=row["status"],
    )


# ── Training (Phase 2 — real implementation in train_model.py) ──


@dataclass
class StartTrainingInput:
    tenant_id: str
    training_job_id: str
    dataset_path: str
    base_model: str
    method: str
    mode: str
    hyperparams: dict
    gpu_class: str | None


@dataclass
class StartTrainingOutput:
    adapter_path: str
    adapter_size_bytes: int
    metrics: dict


# Re-export from real implementation
from src.activities.train_model import start_training  # noqa: E402, F401, I001


# ── Evaluation (Phase 3 — real implementation in run_evaluation.py) ──


@dataclass
class RunEvaluationInput:
    tenant_id: str
    model_id: str
    evaluation_id: str
    adapter_path: str
    base_model: str
    dataset_path: str
    judge_model: str = ""
    judge_api_base: str = ""


@dataclass
class RunEvaluationOutput:
    scores: dict
    report: dict


# Re-export from real implementation
from src.activities.run_evaluation import run_evaluation  # noqa: E402, F401, I001


# ── Deployment (Phase 4) ──


@dataclass
class DeployModelInput:
    tenant_id: str
    model_id: str
    adapter_path: str
    base_model: str
    deployment_config: dict


@dataclass
class DeployModelOutput:
    endpoint_url: str
    deployment_status: str


@activity.defn
async def deploy_model(input: DeployModelInput) -> DeployModelOutput:
    """Deploy a fine-tuned model. Phase 4 implementation."""
    activity.logger.info("Stub: deploy_model for %s", input.model_id)
    return DeployModelOutput(endpoint_url="", deployment_status="pending")
