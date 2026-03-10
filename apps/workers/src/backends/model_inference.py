"""Model inference backend — swap the generation logic without touching evaluation suites.

Protocol: ModelInference
  generate(model, tokenizer, prompt, max_new_tokens) -> str

Built-in backends:
  "hf"  — HuggingFace transformers + PyTorch (default)

Register custom backends with register() to add vLLM, llama-cpp, etc.
"""

from typing import Any, Protocol


class ModelInference(Protocol):
    """Protocol for model text generation backends."""

    def generate(
        self,
        model: Any,
        tokenizer: Any,
        prompt: str,
        max_new_tokens: int = 512,
    ) -> str:
        """Generate a text completion for the given prompt."""
        ...


# -- Implementations --


class HFModelInference:
    """Text generation using HuggingFace transformers + PyTorch (default)."""

    def generate(
        self,
        model: Any,
        tokenizer: Any,
        prompt: str,
        max_new_tokens: int = 512,
    ) -> str:
        import torch

        inputs = tokenizer(prompt, return_tensors="pt", truncation=True, max_length=1536)
        inputs = {k: v.to(model.device) for k, v in inputs.items()}

        with torch.no_grad():
            outputs = model.generate(
                **inputs,
                max_new_tokens=max_new_tokens,
                temperature=0.1,
                do_sample=True,
                pad_token_id=tokenizer.eos_token_id,
            )

        generated_ids = outputs[0][inputs["input_ids"].shape[1] :]
        return tokenizer.decode(generated_ids, skip_special_tokens=True).strip()


# -- Registry & factory --

_REGISTRY: dict[str, type] = {
    "hf": HFModelInference,
}


def register(name: str, cls: type) -> None:
    """Register a custom ModelInference implementation."""
    _REGISTRY[name] = cls


def get(name: str = "hf") -> ModelInference:
    """Instantiate the named ModelInference backend.

    Raises ValueError listing available backends if name is unknown.
    """
    cls = _REGISTRY.get(name)
    if cls is None:
        available = list(_REGISTRY)
        raise ValueError(f"Unknown model_inference backend '{name}'. Available: {available}")
    return cls()
