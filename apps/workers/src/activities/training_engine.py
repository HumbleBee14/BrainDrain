"""TrainingEngine protocol — decouples training logic from ML library internals.

Provides:
  - TrainingEngine Protocol: abstract interface for model loading + adapter setup
  - UnslothEngine: concrete implementation using Unsloth + TRL
  - TrainingStrategy Protocol: abstract interface per training mode
  - Strategy registry: Quick (SFT), Aligned (SFT→DPO), Reasoning (SFT→GRPO)

Services depend on Protocols, not concrete implementations.
Swapping from Unsloth to another library requires only a new TrainingEngine
class registered via register_engine() and selected via APP_TRAINING_ENGINE.
"""

import logging
from pathlib import Path
from typing import TYPE_CHECKING, Any, Protocol

if TYPE_CHECKING:
    from src.config import WorkerSettings

logger = logging.getLogger("platform.training.engine")


# -- TrainingEngine Protocol --


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

    def prepare_for_inference(self, model: Any) -> Any:
        """Prepare model for inference (e.g. kernel swap). Default: identity."""
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

        from src.activities.chat_template import ensure_chat_template

        model, tokenizer = FastLanguageModel.from_pretrained(
            model_name=model_name,
            max_seq_length=max_seq_length,
            load_in_4bit=load_in_4bit,
            dtype=None,
        )
        # Guarantee a chat template so training/eval format exactly as the
        # serving backend will (and persist a fallback for template-less base
        # models, since save_adapter writes this tokenizer alongside the adapter).
        ensure_chat_template(tokenizer)
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

    def prepare_for_inference(self, model: Any) -> Any:
        from unsloth import FastLanguageModel

        FastLanguageModel.for_inference(model)
        return model


class TransformersEngine:
    """Plain `transformers` + `peft` implementation, with no Unsloth.

    Exists because Unsloth cannot share an environment with vLLM — it pins its own
    torch build — and on-policy distillation needs vLLM in-process to reach its
    teacher. So the image that can host a teacher cannot host Unsloth, and this is
    the engine that runs there.

    Slower than `UnslothEngine` and deliberately not a replacement for it: every
    mode that does not need a live teacher keeps the faster path.
    """

    def load_model(
        self,
        model_name: str,
        max_seq_length: int,
        load_in_4bit: bool,
    ) -> tuple[Any, Any]:
        import torch
        from transformers import AutoModelForCausalLM, AutoTokenizer

        from src.activities.chat_template import ensure_chat_template

        kwargs: dict[str, Any] = {"dtype": torch.bfloat16}
        if load_in_4bit:
            from transformers import BitsAndBytesConfig

            kwargs["quantization_config"] = BitsAndBytesConfig(
                load_in_4bit=True,
                bnb_4bit_compute_dtype=torch.bfloat16,
                bnb_4bit_quant_type="nf4",
                bnb_4bit_use_double_quant=True,
            )

        model = AutoModelForCausalLM.from_pretrained(model_name, **kwargs)
        tokenizer = AutoTokenizer.from_pretrained(model_name, model_max_length=max_seq_length)

        # A right-padding tokenizer silently corrupts generation: rollouts would be
        # scored with pad tokens interleaved into the teacher's view of them.
        tokenizer.padding_side = "left"
        if tokenizer.pad_token is None:
            tokenizer.pad_token = tokenizer.eos_token

        ensure_chat_template(tokenizer)
        return model, tokenizer

    def attach_adapter(
        self,
        model: Any,
        r: int = 16,
        lora_alpha: int = 16,
        lora_dropout: int = 0,
        target_modules: list[str] | None = None,
    ) -> Any:
        from peft import LoraConfig, get_peft_model

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

        model.enable_input_require_grads()
        return get_peft_model(
            model,
            LoraConfig(
                r=r,
                lora_alpha=lora_alpha,
                lora_dropout=lora_dropout,
                target_modules=target_modules,
                task_type="CAUSAL_LM",
                bias="none",
            ),
        )

    def save_adapter(self, model: Any, tokenizer: Any, output_dir: Path) -> None:
        model.save_pretrained(str(output_dir))
        tokenizer.save_pretrained(str(output_dir))

    def prepare_for_inference(self, model: Any) -> Any:
        model.eval()
        return model


# -- Engine Registry --

_ENGINE_REGISTRY: dict[str, type] = {
    "unsloth": UnslothEngine,
    "transformers": TransformersEngine,
}


def register_engine(name: str, cls: type) -> None:
    """Register a custom TrainingEngine implementation.

    Example:
        from src.activities.training_engine import register_engine
        register_engine("my_engine", MyCustomEngine)
    Then set APP_TRAINING_ENGINE=my_engine in the environment.
    """
    _ENGINE_REGISTRY[name] = cls


def get_engine(
    settings: "WorkerSettings | None" = None, *, required: str | None = None
) -> TrainingEngine:
    """Instantiate the configured TrainingEngine.

    The engine is selected by settings.training_engine (default: 'unsloth').
    Set APP_TRAINING_ENGINE env var to select a different registered engine.

    `required` overrides configuration for strategies that only work on one
    engine — on-policy distillation cannot run under Unsloth, because its image
    cannot contain Unsloth at all. A strategy declaring its engine is preferable
    to relying on an env var being right in the image it happens to run in.
    """
    name = required or (settings.training_engine if settings else "unsloth")
    cls = _ENGINE_REGISTRY.get(name)
    if cls is None:
        available = list(_ENGINE_REGISTRY)
        raise ValueError(f"Unknown training_engine '{name}'. Available: {available}")
    return cls()


# -- TrainingStrategy Protocol --


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


# -- Strategy Registry --

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
