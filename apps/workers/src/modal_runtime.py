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
    """Create a boto3 S3 client from settings. Returns (client, bucket)."""
    import boto3

    client = boto3.client(
        "s3",
        endpoint_url=settings.s3_endpoint,
        aws_access_key_id=settings.s3_access_key,
        aws_secret_access_key=settings.s3_secret_key,
        region_name=settings.s3_region,
    )
    return client, settings.s3_bucket
