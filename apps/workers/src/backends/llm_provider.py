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

from src.failure_message import NO_LLM_KEY


class LlmApiError(RuntimeError):
    """Provider returned an error status, with its own explanation attached."""

    def __init__(self, status_code: int, detail: str):
        self.status_code = status_code
        self.detail = detail
        suffix = f": {detail}" if detail else ""
        super().__init__(f"LLM provider returned HTTP {status_code}{suffix}")

    @property
    def is_retryable(self) -> bool:
        return self.status_code == 429 or self.status_code >= 500


def _provider_detail(body: str, limit: int = 300) -> str:
    text = (body or "").strip()
    if not text:
        return ""
    try:
        parsed = json.loads(text)
    except ValueError:
        parsed = None
    if isinstance(parsed, dict):
        err = parsed.get("error")
        if isinstance(err, dict) and isinstance(err.get("message"), str):
            text = err["message"]
        elif isinstance(err, str):
            text = err
        elif isinstance(parsed.get("message"), str):
            text = parsed["message"]
    text = text.strip()
    return text if len(text) <= limit else text[:limit] + "..."


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
        # Name the missing setting instead of letting it surface as a bare 401.
        if not (api_key or "").strip():
            raise RuntimeError(NO_LLM_KEY)

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
        if resp.status_code >= 400:
            raise LlmApiError(resp.status_code, _provider_detail(resp.text))

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
