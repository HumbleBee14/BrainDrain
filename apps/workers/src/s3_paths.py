"""S3 path builders matching crates/shared/src/s3_paths.rs.

All paths are tenant-scoped for multi-tenancy isolation.
"""


def upload_path(tenant_id: str, project_id: str, file_id: str, ext: str) -> str:
    return f"uploads/{tenant_id}/{project_id}/{file_id}.{ext}"


def parsed_path(tenant_id: str, project_id: str, doc_id: str) -> str:
    return f"parsed/{tenant_id}/{project_id}/{doc_id}.json"


def dataset_path(tenant_id: str, project_id: str, dataset_id: str) -> str:
    return f"datasets/{tenant_id}/{project_id}/{dataset_id}.jsonl"


def chunks_path(tenant_id: str, project_id: str, batch_id: str) -> str:
    return f"chunks/{tenant_id}/{project_id}/{batch_id}.jsonl"


def pairs_path(tenant_id: str, project_id: str, batch_id: str) -> str:
    return f"pairs/{tenant_id}/{project_id}/{batch_id}.jsonl"


def adapter_prefix(tenant_id: str, model_id: str) -> str:
    return f"adapters/{tenant_id}/{model_id}/"


def adapter_file(tenant_id: str, model_id: str, filename: str) -> str:
    return f"adapters/{tenant_id}/{model_id}/{filename}"


def adapter_training_prefix(tenant_id: str, job_id: str) -> str:
    """Adapter path keyed by training job ID (used at training time before
    the model record exists). See also ``adapter_prefix`` which is keyed
    by model ID for post-training lookups."""
    return f"adapters/{tenant_id}/{job_id}/"


def checkpoint_prefix(tenant_id: str, training_id: str) -> str:
    return f"checkpoints/{tenant_id}/{training_id}/"


def export_path(tenant_id: str, model_id: str, filename: str) -> str:
    return f"exports/{tenant_id}/{model_id}/{filename}"
