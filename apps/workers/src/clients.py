"""Shared infrastructure clients for worker activities.

Thin compatibility layer over the InfraContainer. Activities can use
either the typed container (via `get_container()`) or these convenience
functions (backward-compatible with existing code).

All clients are initialized once at worker startup via init_clients().
"""

import logging

import asyncpg
import redis.asyncio as aioredis

from src.config import WorkerSettings
from src.infra import close_container, get_container, init_container

logger = logging.getLogger("platform.clients")


async def init_clients(settings: WorkerSettings) -> None:
    """Initialize all infrastructure clients. Call once at worker startup."""
    await init_container(settings)


async def close_clients() -> None:
    """Gracefully close all clients. Call at worker shutdown."""
    await close_container()


def get_s3():
    """Get the S3 client."""
    return get_container().s3


def get_s3_bucket() -> str:
    """Get the configured S3 bucket name."""
    return get_container().s3_bucket


async def get_db() -> asyncpg.Pool:
    """Get the database connection pool."""
    return get_container().db


def get_redis() -> aioredis.Redis:
    """Get the async Redis client."""
    return get_container().redis


def get_settings() -> WorkerSettings:
    """Get the worker settings."""
    return get_container().settings
