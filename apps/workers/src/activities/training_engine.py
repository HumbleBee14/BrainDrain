"""TrainingEngine protocol — decouples training logic from ML library internals.

Provides:
  - TrainingEngine Protocol: abstract interface for model loading + adapter setup
  - UnslothEngine: concrete implementation using Unsloth + TRL
  - TrainingStrategy Protocol: abstract interface per training mode
  - Strategy registry: Quick (SFT), Aligned (SFT→DPO), Reasoning (SFT→GRPO)

Services depend on Protocols, not concrete implementations.
Swapping from Unsloth to HuggingFace PEFT or another library
requires only a new TrainingEngine implementation.
"""

import logging
from pathlib import Path
from typing import Any, Protocol

logger = logging.getLogger("platform.training.engine")


# ── TrainingEngine Protocol ─────────────────────────────────────────


class TrainingEngine(Protocol):
    """Protocol for ML model loading and adapter attachment.

    Implement this for any ML framework (Unsloth, PEFT, axolotl, etc.).
    """

    def load_model(
        self,
        model_name: str,
        max_seq_length: int,
        load_in_4bit: bool,
    ) -> tuple[Any, Any]:
        """Load a base model and tokenizer.

        Returns (model, tokenizer) tuple.
        """
        ...

    def attach_adapter(
        self,
        model: Any,
        r: int,
        lora_alpha: int,
        lora_dropout: int,
        target_modules: list[str],
    ) -> Any:
        """Attach LoRA adapters to the model. Returns modified model."""
        ...

    def save_adapter(self, model: Any, tokenizer: Any, output_dir: Path) -> None:
        """Save adapter weights and tokenizer to disk."""
        ...


class UnslothEngine:
    """Unsloth-based TrainingEngine implementation.

    Uses FastLanguageModel for 2x faster training with automatic
    optimizations (fused kernels, memory-efficient attention, etc.).
    """

    def load_model(
        self,
        model_name: str,
        max_seq_length: int,
        load_in_4bit: bool,
    ) -> tuple[Any, Any]:
        from unsloth import FastLanguageModel

        model, tokenizer = FastLanguageModel.from_pretrained(
            model_name=model_name,
            max_seq_length=max_seq_length,
            load_in_4bit=load_in_4bit,
            dtype=None,
        )
        return model, tokenizer

    def attach_adapter(
        self,
        model: Any,
        r: int = 16,
        lora_alpha: int = 16,
        lora_dropout: int = 0,
        target_modules: list[str] | None = None,
    ) -> Any:
        from unsloth import FastLanguageModel

        if target_modules is None:
            target_modules = [
                "q_proj",
                "k_proj",
                "v_proj",
                "o_proj",
                "gate_proj",
                "up_proj",
                "down_proj",
            ]

        return FastLanguageModel.get_peft_model(
            model,
            r=r,
            lora_alpha=lora_alpha,
            lora_dropout=lora_dropout,
            target_modules=target_modules,
            use_gradient_checkpointing="unsloth",
        )

    def save_adapter(self, model: Any, tokenizer: Any, output_dir: Path) -> None:
        model.save_pretrained(str(output_dir))
        tokenizer.save_pretrained(str(output_dir))


# ── TrainingStrategy Protocol ───────────────────────────────────────


class TrainingStrategy(Protocol):
    """Protocol for a training mode (quick, aligned, reasoning, iterative).

    Each strategy knows how to run its specific training pipeline
    given a prepared model and dataset.
    """

    @property
    def name(self) -> str:
        """Human-readable strategy name."""
        ...

    def execute(
        self,
        model: Any,
        tokenizer: Any,
        dataset: Any,
        hp: dict,
        job_id: str,
        max_seq_length: int,
        **kwargs: Any,
    ) -> dict:
        """Run the training pipeline. Returns metrics dict."""
        ...


# ── Strategy Registry ───────────────────────────────────────────────

_STRATEGY_REGISTRY: dict[str, type] = {}


def register_strategy(mode: str):
    """Decorator to register a TrainingStrategy class for a training mode."""

    def decorator(cls: type) -> type:
        _STRATEGY_REGISTRY[mode] = cls
        return cls

    return decorator


def get_strategy(mode: str) -> TrainingStrategy:
    """Look up and instantiate the strategy for a given training mode."""
    cls = _STRATEGY_REGISTRY.get(mode)
    if cls is None:
        available = ", ".join(sorted(_STRATEGY_REGISTRY.keys()))
        raise ValueError(f"Unknown training mode: '{mode}'. Available: {available}")
    return cls()


def get_engine() -> UnslothEngine:
    """Get the default training engine. Swap this to change ML backend."""
    return UnslothEngine()
