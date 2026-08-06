"""Circuit breaker for external API calls (LLM, vLLM, etc.).

Provides a Protocol-based abstraction so the concrete implementation
can be swapped without changing activity code.

Usage in activities:
    result = await infra.circuit_breaker.call(some_async_fn, arg1, arg2)
"""

import logging
import threading
import time
from collections.abc import Callable
from typing import Any, Protocol, runtime_checkable

logger = logging.getLogger("platform.circuit_breaker")


def _counts_as_outage(exc: Exception) -> bool:
    """A breaker guards against an unhealthy dependency, not against our own
    malformed request. An error that reports itself as non-retryable (a 4xx such
    as an unknown model) would otherwise trip the breaker and block every caller.

    ValueError is our own configuration surface (missing API key, bad settings):
    tripping the breaker on it would replace the actionable message with a
    generic "provider unavailable" for every caller.
    """
    if isinstance(exc, ValueError):
        return False
    retryable = getattr(exc, "is_retryable", None)
    return retryable is not False


class CircuitBreakerOpen(Exception):
    """Raised when the circuit breaker is open and rejecting calls."""


@runtime_checkable
class CircuitBreakerPolicy(Protocol):
    """Protocol for circuit breaker implementations.

    Swap the concrete implementation by providing a new class
    that satisfies this Protocol.
    """

    async def call(self, func: Callable, *args: Any, **kwargs: Any) -> Any:
        """Execute func through the circuit breaker."""
        ...

    @property
    def state(self) -> str:
        """Current state: 'closed', 'open', or 'half_open'."""
        ...


class NoOpCircuitBreaker:
    """Pass-through implementation for testing or when disabled."""

    @property
    def state(self) -> str:
        return "closed"

    async def call(self, func: Callable, *args: Any, **kwargs: Any) -> Any:
        return await func(*args, **kwargs)


class AsyncCircuitBreaker:
    """Async-compatible circuit breaker.

    Opens after `fail_max` consecutive failures. Allows a single
    probe call after `reset_timeout` seconds (half-open state).
    Thread-safe via lock.
    """

    def __init__(self, name: str, fail_max: int = 5, reset_timeout: int = 30):
        self._name = name
        self._fail_max = fail_max
        self._reset_timeout = reset_timeout
        self._failures = 0
        self._state = "closed"
        self._opened_at: float = 0
        self._last_failure: str = ""
        self._lock = threading.Lock()

    @property
    def state(self) -> str:
        with self._lock:
            if self._state == "open":
                if time.monotonic() - self._opened_at >= self._reset_timeout:
                    self._state = "half_open"
            return self._state

    async def call(self, func: Callable, *args: Any, **kwargs: Any) -> Any:
        current = self.state

        if current == "open":
            cause = f" Last error: {self._last_failure}" if self._last_failure else ""
            raise CircuitBreakerOpen(
                f"The {self._name} provider is temporarily unavailable after repeated "
                f"failures. Retry in about {self._reset_timeout} seconds.{cause}"
            )

        try:
            result = await func(*args, **kwargs)
        except Exception as exc:
            if _counts_as_outage(exc):
                self._record_failure(exc)
            raise

        self._record_success()
        return result

    def _record_failure(self, exc: Exception) -> None:
        with self._lock:
            self._failures += 1
            self._last_failure = str(exc)[:300]
            if self._failures >= self._fail_max:
                old = self._state
                self._state = "open"
                self._opened_at = time.monotonic()
                if old != "open":
                    logger.warning(
                        "Circuit breaker '%s' opened after %d failures",
                        self._name,
                        self._failures,
                    )

    def _record_success(self) -> None:
        with self._lock:
            if self._state == "half_open":
                logger.info("Circuit breaker '%s' closed (recovered)", self._name)
            self._failures = 0
            self._state = "closed"


def create_circuit_breaker(
    name: str,
    enabled: bool,
    fail_max: int = 5,
    reset_timeout: int = 30,
) -> CircuitBreakerPolicy:
    """Factory function to create the appropriate circuit breaker."""
    if not enabled:
        return NoOpCircuitBreaker()
    return AsyncCircuitBreaker(name=name, fail_max=fail_max, reset_timeout=reset_timeout)
