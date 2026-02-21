"""Tests for worker configuration validation."""
import pytest
from pydantic import ValidationError

from src.config import WorkerSettings


class TestWorkerSettings:
    def test_default_settings(self):
        """Default settings should have valid values."""
        settings = WorkerSettings()
        assert settings.temporal_address
        assert settings.temporal_namespace

    def test_s3_bucket_has_default(self):
        settings = WorkerSettings()
        assert settings.s3_bucket

    def test_temporal_task_queue_has_default(self):
        settings = WorkerSettings()
        assert settings.temporal_task_queue == "ml-pipeline"

    def test_database_url_default_is_postgresql(self):
        settings = WorkerSettings()
        assert settings.database_url.startswith("postgresql://")

    def test_s3_endpoint_default_is_http(self):
        settings = WorkerSettings()
        assert settings.s3_endpoint.startswith("http://") or settings.s3_endpoint.startswith(
            "https://"
        )

    def test_invalid_database_url_rejected(self):
        with pytest.raises(ValidationError):
            WorkerSettings(database_url="mysql://localhost/db")

    def test_invalid_s3_endpoint_rejected(self):
        with pytest.raises(ValidationError):
            WorkerSettings(s3_endpoint="ftp://localhost:9000")

    def test_empty_temporal_address_rejected(self):
        with pytest.raises(ValidationError):
            WorkerSettings(temporal_address="   ")

    def test_empty_s3_bucket_rejected(self):
        with pytest.raises(ValidationError):
            WorkerSettings(s3_bucket="   ")

    def test_invalid_llm_api_base_url_rejected(self):
        with pytest.raises(ValidationError):
            WorkerSettings(llm_api_base_url="not-a-url")

    def test_worker_mode_default(self):
        settings = WorkerSettings()
        assert settings.worker_mode == "all"

    def test_log_level_default(self):
        settings = WorkerSettings()
        assert settings.log_level == "INFO"
