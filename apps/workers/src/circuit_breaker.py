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
            raise CircuitBreakerOpen(f"Circuit breaker '{self._name}' is open")

        try:
            result = await func(*args, **kwargs)
        except Exception:
            self._record_failure()
            raise

        self._record_success()
        return result

    def _record_failure(self) -> None:
        with self._lock:
            self._failures += 1
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
