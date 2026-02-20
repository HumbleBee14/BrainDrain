"""Shared infrastructure clients for worker activities.

Initialized once at worker startup and accessed by activities
via module-level references. All clients are configured from WorkerSettings.
"""

import logging

import asyncpg
import boto3
import redis.asyncio as aioredis

from src.config import WorkerSettings

logger = logging.getLogger("platform.clients")

# Module-level client references, initialized by init_clients()
_s3_client: boto3.client | None = None
_db_pool: asyncpg.Pool | None = None
_redis: aioredis.Redis | None = None
_settings: WorkerSettings | None = None


async def init_clients(settings: WorkerSettings) -> None:
    """Initialize all infrastructure clients. Call once at worker startup."""
    global _s3_client, _db_pool, _redis, _settings  # noqa: PLW0603
    _settings = settings

    # S3 (boto3 is synchronous — fine for Temporal activities which run in thread pool)
    _s3_client = boto3.client(
        "s3",
        endpoint_url=settings.s3_endpoint,
        aws_access_key_id=settings.s3_access_key,
        aws_secret_access_key=settings.s3_secret_key,
        region_name=settings.s3_region,
    )
    logger.info("S3 client initialized (endpoint: %s)", settings.s3_endpoint)

    # PostgreSQL
    _db_pool = await asyncpg.create_pool(
        settings.database_url,
        min_size=2,
        max_size=10,
    )
    logger.info("PostgreSQL pool initialized")

    # Redis
    _redis = aioredis.from_url(settings.redis_url)
    logger.info("Redis client initialized")


async def close_clients() -> None:
    """Gracefully close all clients. Call at worker shutdown."""
    global _db_pool, _redis  # noqa: PLW0603
    if _db_pool:
        await _db_pool.close()
        _db_pool = None
    if _redis:
        await _redis.aclose()
        _redis = None


def get_s3() -> boto3.client:
    if _s3_client is None:
        raise RuntimeError("S3 client not initialized. Call init_clients() first.")
    return _s3_client


def get_s3_bucket() -> str:
    if _settings is None:
        raise RuntimeError("Settings not initialized. Call init_clients() first.")
    return _settings.s3_bucket


async def get_db() -> asyncpg.Pool:
    if _db_pool is None:
        raise RuntimeError("DB pool not initialized. Call init_clients() first.")
    return _db_pool


def get_redis() -> aioredis.Redis:
    if _redis is None:
        raise RuntimeError("Redis client not initialized. Call init_clients() first.")
    return _redis


def get_settings() -> WorkerSettings:
    if _settings is None:
        raise RuntimeError("Settings not initialized. Call init_clients() first.")
    return _settings
