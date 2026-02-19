"""Stub activities for the ML pipeline.

Each activity represents a discrete unit of work. In Phase 1+, these will
be replaced with actual implementations using Unsloth, distilabel, etc.
All activities are designed to be idempotent — safe to retry on failure.
"""

from dataclasses import dataclass

from temporalio import activity


@dataclass
class ParseDocumentInput:
    tenant_id: str
    document_id: str
    storage_path: str
    mime_type: str


@dataclass
class ParseDocumentOutput:
    page_count: int
    language: str | None
    parse_quality: float


@activity.defn
async def parse_document(input: ParseDocumentInput) -> ParseDocumentOutput:
    """Parse an uploaded document (PDF, DOCX, etc.) into structured text.

    Phase 1 implementation: MinerU for PDF, python-docx for DOCX, etc.
    """
    activity.logger.info("Stub: parse_document for %s", input.document_id)
    return ParseDocumentOutput(page_count=0, language=None, parse_quality=0.0)


@dataclass
class GenerateSyntheticPairsInput:
    tenant_id: str
    project_id: str
    document_ids: list[str]
    task_type: str
    config: dict


@dataclass
class GenerateSyntheticPairsOutput:
    pair_count: int
    storage_path: str


@activity.defn
async def generate_synthetic_pairs(
    input: GenerateSyntheticPairsInput,
) -> GenerateSyntheticPairsOutput:
    """Generate instruction/response pairs from parsed documents.

    Phase 1 implementation: distilabel pipelines with LLM-as-judge.
    """
    activity.logger.info("Stub: generate_synthetic_pairs for %s", input.project_id)
    return GenerateSyntheticPairsOutput(pair_count=0, storage_path="")


@dataclass
class BuildDatasetInput:
    tenant_id: str
    project_id: str
    dataset_id: str
    format: str
    config: dict


@dataclass
class BuildDatasetOutput:
    pair_count: int
    storage_path: str


@activity.defn
async def build_dataset(input: BuildDatasetInput) -> BuildDatasetOutput:
    """Build a training-ready dataset from refined pairs.

    Phase 1 implementation: HuggingFace datasets formatting,
    train/val split, chat template application.
    """
    activity.logger.info("Stub: build_dataset for %s", input.dataset_id)
    return BuildDatasetOutput(pair_count=0, storage_path="")


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


@activity.defn
async def start_training(input: StartTrainingInput) -> StartTrainingOutput:
    """Run the fine-tuning job using Unsloth / TRL.

    Phase 1 implementation: Unsloth FastModel for LoRA/QLoRA,
    TRL SFTTrainer, W&B metrics logging.
    """
    activity.logger.info("Stub: start_training for %s", input.training_job_id)
    return StartTrainingOutput(adapter_path="", adapter_size_bytes=0, metrics={})


@dataclass
class RunEvaluationInput:
    tenant_id: str
    model_id: str
    evaluation_id: str
    adapter_path: str
    base_model: str
    dataset_path: str


@dataclass
class RunEvaluationOutput:
    scores: dict
    report: dict


@activity.defn
async def run_evaluation(input: RunEvaluationInput) -> RunEvaluationOutput:
    """Evaluate a fine-tuned model against held-out data.

    Phase 1 implementation: LLM-as-judge evaluation,
    task-specific metrics (BLEU, ROUGE, accuracy).
    """
    activity.logger.info("Stub: run_evaluation for %s", input.evaluation_id)
    return RunEvaluationOutput(scores={}, report={})


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
    """Deploy a fine-tuned model to an inference endpoint.

    Phase 1 implementation: vLLM with LoRA adapter loading.
    """
    activity.logger.info("Stub: deploy_model for %s", input.model_id)
    return DeployModelOutput(endpoint_url="", deployment_status="pending")
