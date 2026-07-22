"""Bootstrap helpers for the remote Modal GPU container.

The remote container runs the pure-compute training core. It needs S3 + LLM
config from env (provided via a Modal secret) but NEVER Postgres/Redis — those
stay on the worker side. Keep this module dependency-light and DB-free.
"""

from src.config import WorkerSettings


def build_settings() -> WorkerSettings:
    """Load WorkerSettings from the container env (populated by the Modal secret)."""
    return WorkerSettings()


def build_s3_client(settings: WorkerSettings):
    """Create a boto3 S3 client from settings. Returns (client, bucket).

    Uses the shared backend-agnostic factory so the remote container talks to
    whatever S3-compatible store is configured (AWS/MinIO/R2) identically to
    the worker side.
    """
    from src.s3_client import create_s3_client

    return create_s3_client(settings), settings.s3_bucket
