"""Infrastructure container with Protocol-based dependency injection.

Provides typed Protocols for all infrastructure dependencies (S3, DB, Redis)
and a concrete container that holds initialized clients. Activities receive
the container via Temporal's dependency injection instead of reaching for
module-level globals.

Usage in activities:
    from src.infra import InfraContainer
    # Injected by Temporal activity context, or accessed via clients module
    container = get_container()
    s3 = container.s3
    db = container.db
"""

import logging
from typing import Protocol, runtime_checkable

import asyncpg
import redis.asyncio as aioredis

from src.circuit_breaker import CircuitBreakerPolicy, create_circuit_breaker
from src.config import WorkerSettings

logger = logging.getLogger("platform.infra")


@runtime_checkable
class ObjectStore(Protocol):
    """Protocol for S3-compatible object storage operations."""

    def download_file(self, bucket: str, key: str, local_path: str) -> None: ...
    def upload_file(self, local_path: str, bucket: str, key: str) -> None: ...
    def get_paginator(self, operation: str): ...


@runtime_checkable
class Database(Protocol):
    """Protocol for async database operations."""

    async def execute(self, query: str, *args) -> str: ...
    async def fetch(self, query: str, *args) -> list: ...
    async def fetchrow(self, query: str, *args): ...


class InfraContainer:
    """Concrete infrastructure container holding all initialized clients.

    Created once at worker startup, passed to activities via the
    module-level accessor (backwards-compatible with existing code).
    """

    def __init__(
        self,
        s3: ObjectStore,
        db: asyncpg.Pool,
        redis: aioredis.Redis,
        settings: WorkerSettings,
        circuit_breaker: CircuitBreakerPolicy,
    ):
        self.s3 = s3
        self.db = db
        self.redis = redis
        self.settings = settings
        self.circuit_breaker = circuit_breaker

    @property
    def s3_bucket(self) -> str:
        return self.settings.s3_bucket


# Module-level container reference (set by init_container)
_container: InfraContainer | None = None


async def init_container(settings: WorkerSettings) -> InfraContainer:
    """Initialize the infrastructure container. Call once at worker startup."""
    global _container  # noqa: PLW0603

    import boto3

    s3 = boto3.client(
        "s3",
        endpoint_url=settings.s3_endpoint,
        aws_access_key_id=settings.s3_access_key,
        aws_secret_access_key=settings.s3_secret_key,
        region_name=settings.s3_region,
    )
    logger.info("S3 client initialized (endpoint: %s)", settings.s3_endpoint)

    db = await asyncpg.create_pool(
        settings.database_url,
        min_size=settings.db_pool_min,
        max_size=settings.db_pool_max,
    )
    logger.info("PostgreSQL pool initialized")

    redis = aioredis.from_url(settings.redis_url)
    logger.info("Redis client initialized")

    llm_breaker = create_circuit_breaker(
        name="llm-api",
        enabled=settings.circuit_breaker_enabled,
        fail_max=settings.circuit_breaker_fail_max,
        reset_timeout=settings.circuit_breaker_reset_timeout,
    )
    logger.info(
        "Circuit breaker initialized (enabled: %s, fail_max: %d)",
        settings.circuit_breaker_enabled,
        settings.circuit_breaker_fail_max,
    )

    _container = InfraContainer(
        s3=s3, db=db, redis=redis, settings=settings, circuit_breaker=llm_breaker
    )
    return _container


async def close_container() -> None:
    """Gracefully close all clients."""
    global _container  # noqa: PLW0603
    if _container is not None:
        if _container.db:
            await _container.db.close()
        if _container.redis:
            await _container.redis.aclose()
        _container = None


def get_container() -> InfraContainer:
    """Get the initialized infrastructure container."""
    if _container is None:
        raise RuntimeError("Infrastructure not initialized. Call init_container() first.")
    return _container


def set_container_for_testing(container: InfraContainer) -> None:
    """Override the global container for tests. Not for production use."""
    global _container  # noqa: PLW0603
    _container = container
