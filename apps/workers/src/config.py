from pydantic import field_validator
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

    # Training / ML
    hf_token: str = ""
    model_cache_dir: str = "/tmp/model_cache"
    worker_mode: str = "all"  # "all" | "main" | "gpu"

    # Logging
    log_level: str = "INFO"

    model_config = {"env_prefix": "APP_", "env_file": ".env"}

    @field_validator("temporal_address")
    @classmethod
    def temporal_address_not_empty(cls, v: str) -> str:
        if not v.strip():
            raise ValueError("temporal_address must not be empty")
        return v

    @field_validator("database_url")
    @classmethod
    def database_url_must_be_postgresql(cls, v: str) -> str:
        if not v.startswith("postgresql://"):
            raise ValueError("database_url must start with 'postgresql://'")
        return v

    @field_validator("s3_endpoint")
    @classmethod
    def s3_endpoint_must_be_http(cls, v: str) -> str:
        if not v.startswith("http://") and not v.startswith("https://"):
            raise ValueError("s3_endpoint must start with 'http://' or 'https://'")
        return v

    @field_validator("llm_api_base_url")
    @classmethod
    def llm_api_base_url_must_be_http(cls, v: str) -> str:
        if not v.startswith("http://") and not v.startswith("https://"):
            raise ValueError("llm_api_base_url must start with 'http://' or 'https://'")
        return v

    @field_validator("s3_bucket")
    @classmethod
    def s3_bucket_not_empty(cls, v: str) -> str:
        if not v.strip():
            raise ValueError("s3_bucket must not be empty")
        return v
