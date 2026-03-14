"""LLM synthesis backend — swap the API provider for pair generation.

Protocol: LLMProvider
  generate(prompt, model, api_base_url, api_key, max_tokens, temperature) -> str

Built-in backends:
  "openai"  — Any OpenAI-compatible API (default)

Register custom backends with register().
"""

import json
from typing import Protocol

import httpx


class LLMProvider(Protocol):
    """Protocol for LLM API providers used in synthetic data generation."""

    async def generate(
        self,
        http: httpx.AsyncClient,
        prompt: str,
        *,
        model: str,
        api_base_url: str,
        api_key: str,
        max_tokens: int = 2000,
        temperature: float = 0.7,
    ) -> str:
        """Send a prompt and return the raw text response."""
        ...


# -- Implementations --


class OpenAICompatibleProvider:
    """Any OpenAI-compatible API (OpenAI, Groq, Together, Ollama, vLLM, etc.)."""

    async def generate(
        self,
        http: httpx.AsyncClient,
        prompt: str,
        *,
        model: str,
        api_base_url: str,
        api_key: str,
        max_tokens: int = 2000,
        temperature: float = 0.7,
    ) -> str:
        url = f"{api_base_url.rstrip('/')}/chat/completions"
        resp = await http.post(
            url,
            headers={"Authorization": f"Bearer {api_key}"},
            json={
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "max_tokens": max_tokens,
                "temperature": temperature,
            },
        )
        resp.raise_for_status()

        data = resp.json()
        content = data["choices"][0]["message"]["content"].strip()

        # Strip markdown code block wrapping if present
        if content.startswith("```"):
            lines = content.split("\n")
            content = "\n".join(lines[1:-1]) if len(lines) > 2 else content

        return content


# -- Helpers --


def parse_pairs_json(raw: str) -> list[dict]:
    """Parse a JSON array of pairs from raw LLM output.

    Markdown code block stripping is handled by LLMProvider.generate(),
    so this receives clean JSON. Raises json.JSONDecodeError on invalid input.
    """
    return json.loads(raw)


# -- Registry & factory --

_REGISTRY: dict[str, type] = {
    "openai": OpenAICompatibleProvider,
}


def register(name: str, cls: type) -> None:
    """Register a custom LLMProvider implementation."""
    _REGISTRY[name] = cls


def get(name: str) -> LLMProvider:
    """Instantiate the named LLMProvider."""
    cls = _REGISTRY.get(name)
    if cls is None:
        available = list(_REGISTRY)
        raise ValueError(f"Unknown llm_provider_backend '{name}'. Available: {available}")
    return cls()
