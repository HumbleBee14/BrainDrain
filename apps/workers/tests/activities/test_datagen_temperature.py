"""The faithfulness judge must sample near-deterministically, independent of
the creative generation temperature. These tests pin the temperature each
closure forwards to the provider."""

import httpx
import pytest

from src.activities.datagen_activities import _llm_call_closure


class _RecordingProvider:
    def __init__(self):
        self.temperature = None

    async def generate(
        self, http, prompt, *, model, api_base_url, api_key, max_tokens, temperature
    ):
        self.temperature = temperature
        return "ok"


class _PassthroughBreaker:
    async def call(self, func, *args, **kwargs):
        return await func(*args, **kwargs)


class _FakeInfra:
    def __init__(self):
        self.circuit_breaker = _PassthroughBreaker()


class _FakeLlmConfig:
    model = "m"
    api_base_url = "http://example"
    api_key = "k"
    max_tokens = 100


@pytest.mark.asyncio
@pytest.mark.parametrize("temperature", [0.0, 0.7])
async def test_closure_forwards_its_bound_temperature(temperature):
    provider = _RecordingProvider()
    async with httpx.AsyncClient() as http:
        call = _llm_call_closure(_FakeInfra(), _FakeLlmConfig(), provider, http, temperature)
        await call("prompt")
    assert provider.temperature == temperature
