"""Tests for worker configuration validation."""

import pytest
from pydantic import ValidationError

from src.config import WorkerSettings


def _make_settings(**overrides) -> WorkerSettings:
    """Create WorkerSettings with test defaults, ignoring env vars and .env files."""
    defaults = {
        "temporal_address": "localhost:7233",
        "temporal_namespace": "default",
        "temporal_task_queue": "ml-pipeline",
        "database_url": "postgresql://test:test@localhost:5432/test",
        "redis_url": "redis://localhost:6379",
        "s3_endpoint": "http://localhost:9000",
        "s3_access_key": "testkey",
        "s3_secret_key": "testsecret",
        "s3_bucket": "test-bucket",
        "s3_region": "us-east-1",
        "llm_api_base_url": "https://api.openai.com/v1",
    }
    defaults.update(overrides)
    return WorkerSettings(_env_file=None, **defaults)


class TestWorkerSettings:
    def test_default_settings(self):
        """Settings with required fields should have valid values."""
        settings = _make_settings()
        assert settings.temporal_address
        assert settings.temporal_namespace

    def test_s3_bucket_has_value(self):
        settings = _make_settings()
        assert settings.s3_bucket == "test-bucket"

    def test_temporal_task_queue_default(self):
        settings = _make_settings()
        assert settings.temporal_task_queue == "ml-pipeline"

    def test_database_url_is_postgresql(self):
        settings = _make_settings()
        assert settings.database_url.startswith("postgresql://")

    def test_s3_endpoint_is_http(self):
        settings = _make_settings()
        assert settings.s3_endpoint.startswith("http://")

    def test_invalid_database_url_rejected(self):
        with pytest.raises(ValidationError):
            _make_settings(database_url="mysql://localhost/db")

    def test_invalid_s3_endpoint_rejected(self):
        with pytest.raises(ValidationError):
            _make_settings(s3_endpoint="ftp://localhost:9000")

    def test_empty_temporal_address_rejected(self):
        with pytest.raises(ValidationError):
            _make_settings(temporal_address="   ")

    def test_empty_s3_bucket_rejected(self):
        with pytest.raises(ValidationError):
            _make_settings(s3_bucket="   ")

    def test_invalid_llm_api_base_url_rejected(self):
        with pytest.raises(ValidationError):
            _make_settings(llm_api_base_url="not-a-url")

    def test_worker_mode_default(self):
        settings = _make_settings()
        assert settings.worker_mode == "all"

    def test_log_level_default(self):
        settings = _make_settings()
        assert settings.log_level == "INFO"

    def test_min_billable_seconds_default(self):
        settings = _make_settings()
        assert settings.min_billable_seconds == 300

    def test_timeout_defaults(self):
        settings = _make_settings()
        assert settings.timeout_parse_minutes == 10
        assert settings.timeout_train_hours == 6
        assert settings.timeout_eval_hours == 1
