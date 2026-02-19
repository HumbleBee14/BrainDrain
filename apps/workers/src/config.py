from pydantic_settings import BaseSettings


class WorkerSettings(BaseSettings):
    """Worker configuration loaded from environment variables."""

    temporal_address: str = "localhost:7233"
    temporal_namespace: str = "default"
    temporal_task_queue: str = "ml-pipeline"

    database_url: str = "postgresql://platform:platform@localhost:5432/platform"
    redis_url: str = "redis://localhost:6379"
    s3_endpoint: str = "http://localhost:9000"
    s3_access_key: str = "minioadmin"
    s3_secret_key: str = "minioadmin"
    s3_bucket: str = "platform"

    log_level: str = "INFO"

    model_config = {"env_prefix": "APP_", "env_file": ".env"}
