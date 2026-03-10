"""Metrics collection backend — swap the sink without touching training code.

Protocol: MetricsCollector
  record(stream_key, data, maxlen) -> None
  close() -> None

Built-in backends:
  "redis"  — Redis streams (default, real-time dashboard)
  "log"    — Python logger (local dev, no Redis required)
  "null"   — no-op (unit tests)

Register custom backends with register().
"""

import logging
from typing import Protocol

logger = logging.getLogger("platform.metrics")


class MetricsCollector(Protocol):
    """Protocol for training metrics collection backends."""

    def record(self, stream_key: str, data: dict[str, str], maxlen: int = 10000) -> None:
        """Emit a metrics event to the configured sink."""
        ...

    def close(self) -> None:
        """Flush and release resources."""
        ...


# -- Implementations --


class RedisCollector:
    """Metrics collection via Redis streams (default).

    Uses a synchronous Redis client — training callbacks run in a thread,
    not in an async context, so async Redis is not appropriate here.
    """

    def __init__(self, redis_url: str) -> None:
        import redis as sync_redis

        self._client = sync_redis.from_url(redis_url)

    def record(self, stream_key: str, data: dict[str, str], maxlen: int = 10000) -> None:
        self._client.xadd(stream_key, data, maxlen=maxlen)

    def close(self) -> None:
        self._client.close()


class LogCollector:
    """Metrics collection via Python logger. No external dependencies required."""

    def record(self, stream_key: str, data: dict[str, str], maxlen: int = 10000) -> None:
        logger.info("metrics stream=%s %s", stream_key, data)

    def close(self) -> None:
        pass


class NullCollector:
    """No-op metrics collector. Useful for unit tests."""

    def record(self, stream_key: str, data: dict[str, str], maxlen: int = 10000) -> None:
        pass

    def close(self) -> None:
        pass


# -- Registry & factory --

_REGISTRY: dict[str, type] = {
    "redis": RedisCollector,
    "log": LogCollector,
    "null": NullCollector,
}


def register(name: str, cls: type) -> None:
    """Register a custom MetricsCollector implementation."""
    _REGISTRY[name] = cls


def get(name: str, redis_url: str = "") -> MetricsCollector:
    """Instantiate the named MetricsCollector.

    Raises ValueError listing available backends if name is unknown.
    """
    cls = _REGISTRY.get(name)
    if cls is None:
        available = list(_REGISTRY)
        raise ValueError(f"Unknown metrics_backend '{name}'. Available: {available}")
    if name == "redis":
        if not redis_url:
            raise ValueError("redis_url is required for the 'redis' metrics backend")
        return cls(redis_url)
    return cls()
