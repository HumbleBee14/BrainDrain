"""LLM Judge backend — swap the judge provider for evaluation and training.

Protocol: LLMJudge (re-exported from llm_judge.py for consistency)
Built-in backends:
  "openai" — OpenAICompatibleJudge (default, any OpenAI-compatible API)

Register custom backends with register().
"""

from src.activities.llm_judge import LLMJudge, OpenAICompatibleJudge

# -- Registry & factory --

_REGISTRY: dict[str, type] = {
    "openai": OpenAICompatibleJudge,
}


def register(name: str, cls: type) -> None:
    """Register a custom LLMJudge implementation."""
    _REGISTRY[name] = cls


def get(
    name: str,
    api_base: str,
    api_key: str,
    model: str,
    max_retries: int = 3,
    on_failure: str = "error",
    **kwargs,
) -> LLMJudge:
    """Instantiate the named LLMJudge with credentials + failure policy.

    Extra keyword arguments (completion budget, timeout, thinking mode) pass
    through to the backend so new judge knobs never require a factory change.
    """
    cls = _REGISTRY.get(name)
    if cls is None:
        available = list(_REGISTRY)
        raise ValueError(f"Unknown judge_backend '{name}'. Available: {available}")
    return cls(api_base, api_key, model, max_retries=max_retries, on_failure=on_failure, **kwargs)


__all__ = ["LLMJudge", "register", "get"]
