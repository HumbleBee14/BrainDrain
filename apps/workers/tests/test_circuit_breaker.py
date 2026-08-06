"""Tests for circuit breaker Protocol and implementations."""

import pytest

from src.circuit_breaker import (
    AsyncCircuitBreaker,
    CircuitBreakerOpen,
    CircuitBreakerPolicy,
    NoOpCircuitBreaker,
    create_circuit_breaker,
)


@pytest.mark.asyncio
async def test_noop_passes_through():
    """NoOpCircuitBreaker should always call the function directly."""
    cb = NoOpCircuitBreaker()
    assert cb.state == "closed"

    async def add(a, b):
        return a + b

    result = await cb.call(add, 2, 3)
    assert result == 5


@pytest.mark.asyncio
async def test_noop_propagates_exceptions():
    """NoOpCircuitBreaker should let exceptions pass through."""
    cb = NoOpCircuitBreaker()

    async def fail():
        raise ValueError("boom")

    with pytest.raises(ValueError, match="boom"):
        await cb.call(fail)


@pytest.mark.asyncio
async def test_noop_satisfies_protocol():
    """NoOpCircuitBreaker must satisfy CircuitBreakerPolicy Protocol."""
    cb = NoOpCircuitBreaker()
    assert isinstance(cb, CircuitBreakerPolicy)


@pytest.mark.asyncio
async def test_async_breaker_satisfies_protocol():
    """AsyncCircuitBreaker must satisfy CircuitBreakerPolicy Protocol."""
    cb = AsyncCircuitBreaker(name="test", fail_max=3, reset_timeout=1)
    assert isinstance(cb, CircuitBreakerPolicy)


@pytest.mark.asyncio
async def test_async_breaker_passes_through_on_success():
    """AsyncCircuitBreaker should call the function when circuit is closed."""
    cb = AsyncCircuitBreaker(name="test-success", fail_max=3, reset_timeout=1)
    assert cb.state == "closed"

    async def ok():
        return 42

    result = await cb.call(ok)
    assert result == 42
    assert cb.state == "closed"


@pytest.mark.asyncio
async def test_circuit_opens_after_failures():
    """Circuit should open after fail_max consecutive failures."""
    cb = AsyncCircuitBreaker(name="test-open", fail_max=2, reset_timeout=60)

    async def fail():
        raise RuntimeError("service down")

    # First two failures should pass through (and be counted)
    with pytest.raises(RuntimeError):
        await cb.call(fail)
    with pytest.raises(RuntimeError):
        await cb.call(fail)

    # Circuit should now be open
    assert cb.state == "open"

    # Third call should be rejected immediately
    with pytest.raises(CircuitBreakerOpen):
        await cb.call(fail)


@pytest.mark.asyncio
async def test_half_open_allows_retry():
    """After reset_timeout, circuit should go half-open and allow a probe."""
    cb = AsyncCircuitBreaker(name="test-half", fail_max=1, reset_timeout=0)

    async def fail():
        raise RuntimeError("down")

    async def ok():
        return "recovered"

    # Trip the breaker
    with pytest.raises(RuntimeError):
        await cb.call(fail)
    assert cb.state != "closed"

    # With reset_timeout=0, it should immediately go half_open
    assert cb.state == "half_open"

    # Successful call in half_open should close the circuit
    result = await cb.call(ok)
    assert result == "recovered"
    assert cb.state == "closed"


@pytest.mark.asyncio
async def test_factory_returns_noop_when_disabled():
    """create_circuit_breaker with enabled=False should return NoOp."""
    cb = create_circuit_breaker(name="test", enabled=False)
    assert isinstance(cb, NoOpCircuitBreaker)


@pytest.mark.asyncio
async def test_factory_returns_async_breaker_when_enabled():
    """create_circuit_breaker with enabled=True should return AsyncCircuitBreaker."""
    cb = create_circuit_breaker(name="test", enabled=True, fail_max=5, reset_timeout=30)
    assert isinstance(cb, AsyncCircuitBreaker)


@pytest.mark.asyncio
async def test_permanent_client_error_does_not_open_the_breaker():
    """A bad model name is our fault, not an outage — it must not block callers."""

    class _Permanent(Exception):
        is_retryable = False

    breaker = AsyncCircuitBreaker(name="llm-api", fail_max=2, reset_timeout=30)

    async def boom():
        raise _Permanent("unknown model")

    for _ in range(5):
        with pytest.raises(_Permanent):
            await breaker.call(boom)

    assert breaker.state == "closed"


@pytest.mark.asyncio
async def test_retryable_error_still_opens_the_breaker():
    class _Transient(Exception):
        is_retryable = True

    breaker = AsyncCircuitBreaker(name="llm-api", fail_max=2, reset_timeout=30)

    async def boom():
        raise _Transient("503")

    for _ in range(2):
        with pytest.raises(_Transient):
            await breaker.call(boom)

    assert breaker.state == "open"


@pytest.mark.asyncio
async def test_unclassified_error_still_opens_the_breaker():
    breaker = AsyncCircuitBreaker(name="llm-api", fail_max=1, reset_timeout=30)

    async def boom():
        raise TimeoutError("no response")

    with pytest.raises(TimeoutError):
        await breaker.call(boom)

    assert breaker.state == "open"


@pytest.mark.asyncio
async def test_open_message_is_user_facing():
    breaker = AsyncCircuitBreaker(name="llm-api", fail_max=1, reset_timeout=30)

    async def boom():
        raise TimeoutError("x")

    with pytest.raises(TimeoutError):
        await breaker.call(boom)

    with pytest.raises(CircuitBreakerOpen, match="temporarily unavailable"):
        await breaker.call(boom)


@pytest.mark.asyncio
async def test_config_value_error_does_not_open_the_breaker():
    breaker = AsyncCircuitBreaker(name="llm-api", fail_max=1, reset_timeout=30)

    async def misconfigured():
        raise ValueError("LLM API key not configured")

    with pytest.raises(ValueError):
        await breaker.call(misconfigured)

    assert breaker.state == "closed"


@pytest.mark.asyncio
async def test_open_message_includes_last_underlying_error():
    breaker = AsyncCircuitBreaker(name="llm-api", fail_max=1, reset_timeout=30)

    async def boom():
        raise TimeoutError("upstream took too long")

    with pytest.raises(TimeoutError):
        await breaker.call(boom)

    with pytest.raises(CircuitBreakerOpen, match="upstream took too long"):
        await breaker.call(boom)
