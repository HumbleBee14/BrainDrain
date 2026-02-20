from pydantic_settings import BaseSettings


class WorkerSettings(BaseSettings):
    """Worker configuration loaded from environment variables."""

    # Temporal
    temporal_address: str = "localhost:7233"
    temporal_namespace: str = "default"
    temporal_task_queue: str = "ml-pipeline"

    # Database
    database_url: str = "postgresql://platform:platform@localhost:5432/platform"

    # Redis
    redis_url: str = "redis://localhost:6379"

    # S3 / Object storage
    s3_endpoint: str = "http://localhost:9000"
    s3_access_key: str = "minioadmin"
    s3_secret_key: str = "minioadmin"
    s3_bucket: str = "platform"
    s3_region: str = "us-east-1"

    # LLM API (OpenAI-compatible — works with any provider)
    llm_api_base_url: str = "https://api.openai.com/v1"
    llm_api_key: str = ""
    llm_model: str = "gpt-4o-mini"
    llm_max_tokens: int = 2000

    # Logging
    log_level: str = "INFO"

    model_config = {"env_prefix": "APP_", "env_file": ".env"}
