"""Stub activities and shared dataclasses for pipeline stages.

Training has a real implementation in train_model.py.
Evaluation and deployment are Phase 3+.
Parsing, data generation, and dataset building have real implementations
in their own modules (parse_document.py, generate_pairs.py, etc.).
"""

from dataclasses import dataclass

from temporalio import activity

from src.infra import InfraContainer

# ── Document info (lightweight DB lookup for workflows) ──


@dataclass
class GetDocumentInfoInput:
    tenant_id: str
    document_id: str


@dataclass
class DocumentInfo:
    document_id: str
    tenant_id: str
    project_id: str
    storage_path: str
    mime_type: str
    status: str


class GetDocumentInfoActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="get_document_info")
    async def run(self, input: GetDocumentInfoInput) -> DocumentInfo:
        db = self.infra.db
        row = await db.fetchrow(
            "SELECT id, tenant_id, project_id, storage_path, mime_type, status "
            "FROM documents WHERE id = $1 AND tenant_id = $2",
            input.document_id,
            input.tenant_id,
        )
        if row is None:
            raise ValueError(f"Document not found: {input.document_id}")
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


# ── Iterative Training (individual round + holdout eval) ──


@dataclass
class TrainSftRoundInput:
    tenant_id: str
    training_job_id: str
    dataset_path: str
    base_model: str
    method: str
    hyperparams: dict
    iteration: int
    adapter_path: str | None  # S3 path to resume from (None for first iteration)
    gpu_class: str | None


@dataclass
class TrainSftRoundOutput:
    adapter_path: str  # S3 path where this iteration's adapter was saved
    adapter_size_bytes: int
    metrics: dict


@dataclass
class EvaluateHoldoutInput:
    tenant_id: str
    training_job_id: str
    adapter_path: str  # S3 path to the adapter to evaluate
    base_model: str
    method: str  # "lora" or "qlora" — determines quantization for model loading
    dataset_path: str  # S3 path to training data (we derive _val.jsonl)
    hyperparams: dict
    iteration: int


@dataclass
class EvaluateHoldoutOutput:
    eval_loss: float
    metrics: dict


# ── Iterative Training DB Lifecycle ──


@dataclass
class FinalizeIterativeTrainingInput:
    tenant_id: str
    training_job_id: str
    base_model: str
    mode: str
    adapter_path: str
    adapter_size_bytes: int
    metrics: dict
    gpu_class: str | None


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


class DeployModelActivity:
    """Deploy a model by calling the Rust API to load the LoRA adapter.

    The Rust API handles the adapter load/unload via the configured inference
    backend (vLLM, TGI, SGLang) and circuit breaker.
    This activity is the Temporal bridge from the full pipeline workflow.
    """

    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="deploy_model")
    async def run(self, input: DeployModelInput) -> DeployModelOutput:
        import aiohttp

        api_url = self.infra.settings.platform_api_url
        token = self.infra.settings.platform_internal_token

        if not token:
            raise RuntimeError(
                "platform_internal_token is not configured; "
                "refusing to call deploy endpoint without authentication"
            )

        activity.logger.info("Deploying model %s (adapter: %s)", input.model_id, input.adapter_path)

        headers = {
            "Content-Type": "application/json",
            "Authorization": f"Bearer {token}",
            "X-Tenant-Id": input.tenant_id,
        }

        async with aiohttp.ClientSession() as session:
            async with session.post(
                f"{api_url}/api/v1/models/{input.model_id}/deploy",
                headers=headers,
                timeout=aiohttp.ClientTimeout(total=300),
            ) as resp:
                if resp.status != 200:
                    body = await resp.text()
                    raise RuntimeError(f"Deploy failed ({resp.status}): {body}")

                data = await resp.json()

        activity.logger.info("Model %s deployed successfully", input.model_id)

        return DeployModelOutput(
            endpoint_url=f"{api_url}/v1/chat/completions",
            deployment_status=data.get("deployment_status", "active"),
        )
