"""On-policy distillation: the student writes, the teacher grades its tokens.

Stages 1 and 2 train on text the teacher wrote. That leaves the student fluent on
the teacher's own sentences and unpractised on its own — the exposure-bias failure
where quality falls apart over long outputs. Here the student generates, and the
teacher scores the tokens the student actually chose, so the correction lands
exactly where the student is going wrong.

The loss is TRL's, not ours, and the reason is worth stating: with a teacher
reachable only as a top-k endpoint, reverse KL can only be evaluated on the
sampled token plus a bucket for everything else. That is a property of the
information available, not of TRL's implementation — a hand-written loss would hit
the same wall for the same money, and Stage 2's own tail term is the same shape.
See docs/distillation/STAGE3-SPIKE-FINDINGS.md §7.

Every knob is resolved from hyperparams with a default here, so a run can be
retuned from configuration without a code change.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass

logger = logging.getLogger("platform.training.on_policy")

# Reverse KL (beta=1.0) is the mode-seeking direction: "never say what the teacher
# would not". That is what a small model needs for generation — forward KL spreads
# its mass and produces the bland, hedging output it is known for.
DEFAULT_BETA = 1.0

# Fully on-policy. Mixing in teacher-written batches (lmbda<1) is a documented
# stabilizer, but Stages 1-2 already provide the off-policy phase, so an improve
# pass that reintroduced it would be repeating work the parent model has done.
DEFAULT_LMBDA = 1.0

# 1.0 = sample the student's own distribution. Lower would sharpen rollouts toward
# greedy text, which is not what the student produces in use, and grading text the
# student would not have written defeats the point of being on-policy.
DEFAULT_TEMPERATURE = 1.0

# The generalized-JSD objective rather than the importance-weighted one. IW-OPD is
# the newer research objective and reachable by configuration; making it the
# default would be a guess ahead of the parity numbers that should decide it.
DEFAULT_OBJECTIVE = "jsd"
SUPPORTED_OBJECTIVES = ("jsd", "iw_opd")

# Forced by the server topology, not chosen: TRL rejects any other value when
# beta>0 with a server teacher, because the endpoint cannot score tokens the
# student picked but the teacher did not rank.
SERVER_REVERSE_KL_TOP_K = 1

DEFAULT_MAX_COMPLETION_LENGTH = 512
DEFAULT_NUM_GENERATIONS = 1

# The trainer's own name for the epoch count, and the platform default behind it.
# Both have to match what the API priced: an on-policy epoch is a full pass of
# student rollouts graded token by token, so training one more than was quoted
# spends a teacher-hour nobody was charged for. Reading a different key entirely —
# which this did — silently trained one epoch against a three-epoch quote.
EPOCHS_HYPERPARAM = "num_train_epochs"
DEFAULT_EPOCHS = 3

# HF `generate` for rollouts rather than colocated vLLM. Slower per token, but it
# cannot compete with the student for GPU memory mid-run, and an out-of-memory kill
# costs the whole container. Flipping this on is the first optimization to measure
# once a run has completed end to end (docs/distillation/STAGE3-TESTING.md).
DEFAULT_USE_VLLM_ROLLOUTS = False


class OnPolicyConfigError(ValueError):
    """A configuration that TRL would reject after a GPU is already paid for.

    Raised as early as possible: these checks mirror `IWOPDConfig.__post_init__`,
    which runs inside the trainer, after the teacher has loaded its weights.
    """


@dataclass(frozen=True)
class OnPolicyPlan:
    """Resolved on-policy settings, ready to hand to TRL."""

    teacher_model: str
    teacher_revision: str | None
    teacher_url: str
    beta: float
    lmbda: float
    temperature: float
    objective: str
    loss_top_k: int
    loss_add_tail: bool
    max_completion_length: int
    num_generations: int
    generation_batch_size: int
    per_device_train_batch_size: int
    gradient_accumulation_steps: int
    use_vllm_rollouts: bool


def _positive_int(hp: dict, key: str, default: int) -> int:
    value = hp.get(key, default)
    try:
        value = int(value)
    except (TypeError, ValueError) as exc:
        raise OnPolicyConfigError(f"{key} must be an integer, got {hp.get(key)!r}") from exc
    if value < 1:
        raise OnPolicyConfigError(f"{key} must be at least 1, got {value}")
    return value


def _unit_float(hp: dict, key: str, default: float) -> float:
    value = hp.get(key, default)
    try:
        value = float(value)
    except (TypeError, ValueError) as exc:
        raise OnPolicyConfigError(f"{key} must be a number, got {hp.get(key)!r}") from exc
    if not 0.0 <= value <= 1.0:
        raise OnPolicyConfigError(f"{key} must be between 0.0 and 1.0, got {value}")
    return value


def resolve_generation_batch_size(
    *,
    per_device_train_batch_size: int,
    gradient_accumulation_steps: int,
    num_generations: int,
    requested: int | None = None,
) -> int:
    """The one batch-size relationship TRL enforces, checked before we pay for a GPU.

    TRL requires
    `generation_batch_size * num_generations == per_device_train_batch_size *
    gradient_accumulation_steps`
    and derives the left side when it is unset. Deriving it the same way here means
    an impossible combination is a validation error at admission rather than a
    crash after a 30B teacher has finished loading.
    """
    sequences = per_device_train_batch_size * gradient_accumulation_steps
    if requested is None:
        if sequences % num_generations:
            raise OnPolicyConfigError(
                f"num_generations ({num_generations}) must divide "
                f"per_device_train_batch_size * gradient_accumulation_steps ({sequences})"
            )
        return sequences // num_generations

    if requested * num_generations != sequences:
        raise OnPolicyConfigError(
            f"generation_batch_size * num_generations ({requested} * {num_generations}) "
            f"must equal per_device_train_batch_size * gradient_accumulation_steps "
            f"({per_device_train_batch_size} * {gradient_accumulation_steps} = {sequences})"
        )
    return requested


def plan_on_policy(
    hp: dict,
    *,
    teacher_model: str,
    teacher_url: str,
    teacher_revision: str | None = None,
) -> OnPolicyPlan:
    """Resolve hyperparams into settings TRL will accept.

    Rejects here everything TRL rejects later, so that a misconfigured run costs
    an API round trip instead of a GPU boot.
    """
    if not teacher_model:
        raise OnPolicyConfigError("on-policy distillation requires a teacher model")
    if not teacher_url:
        raise OnPolicyConfigError("on-policy distillation requires a teacher server url")

    objective = str(hp.get("distill_objective", DEFAULT_OBJECTIVE))
    if objective not in SUPPORTED_OBJECTIVES:
        raise OnPolicyConfigError(
            f"Unknown distill_objective '{objective}'. Available: {', '.join(SUPPORTED_OBJECTIVES)}"
        )

    beta = _unit_float(hp, "distill_beta", DEFAULT_BETA)
    lmbda = _unit_float(hp, "on_policy_lambda", DEFAULT_LMBDA)
    temperature = float(hp.get("rollout_temperature", DEFAULT_TEMPERATURE))
    if temperature <= 0.0:
        raise OnPolicyConfigError(f"rollout_temperature must be positive, got {temperature}")

    if beta <= 0.0:
        raise OnPolicyConfigError(
            "distill_beta must be greater than 0 for on-policy distillation: at 0 the "
            "loss is forward KL, which is what the off-policy stages already do."
        )

    # IW-OPD is defined on sampled tokens from the current policy, so TRL requires
    # fully on-policy data for it. Caught here rather than surfaced as its internal
    # error message, which names parameters this platform does not expose.
    if objective == "iw_opd" and lmbda < 1.0:
        raise OnPolicyConfigError(
            "distill_objective 'iw_opd' requires on_policy_lambda = 1.0, because the "
            f"objective is defined on the student's own samples (got {lmbda})."
        )

    per_device = _positive_int(hp, "per_device_train_batch_size", 1)
    accumulation = _positive_int(hp, "gradient_accumulation_steps", 4)
    num_generations = _positive_int(hp, "num_generations", DEFAULT_NUM_GENERATIONS)
    generation_batch_size = resolve_generation_batch_size(
        per_device_train_batch_size=per_device,
        gradient_accumulation_steps=accumulation,
        num_generations=num_generations,
        requested=(
            _positive_int(hp, "generation_batch_size", 1)
            if hp.get("generation_batch_size") is not None
            else None
        ),
    )

    return OnPolicyPlan(
        teacher_model=teacher_model,
        teacher_revision=teacher_revision,
        teacher_url=teacher_url,
        beta=beta,
        lmbda=lmbda,
        temperature=temperature,
        objective=objective,
        loss_top_k=SERVER_REVERSE_KL_TOP_K,
        loss_add_tail=bool(hp.get("distill_add_tail", True)),
        max_completion_length=_positive_int(
            hp, "max_completion_length", DEFAULT_MAX_COMPLETION_LENGTH
        ),
        num_generations=num_generations,
        generation_batch_size=generation_batch_size,
        per_device_train_batch_size=per_device,
        gradient_accumulation_steps=accumulation,
        use_vllm_rollouts=bool(hp.get("use_vllm_rollouts", DEFAULT_USE_VLLM_ROLLOUTS)),
    )


def trainer_config_kwargs(plan: OnPolicyPlan, *, output_dir: str, hp: dict) -> dict:
    """Keyword arguments for TRL's on-policy config.

    Kept as a plain dict, separate from constructing it, so the mapping can be
    tested without the trainer — and therefore without a GPU, vLLM, or the
    experimental TRL namespace being importable.
    """
    kwargs = {
        "output_dir": output_dir,
        "teacher_model_name_or_path": plan.teacher_model,
        "use_teacher_server": True,
        "teacher_model_server_url": plan.teacher_url,
        "distillation_objective": plan.objective,
        "beta": plan.beta,
        "lmbda": plan.lmbda,
        "temperature": plan.temperature,
        "loss_top_k": plan.loss_top_k,
        "loss_add_tail": plan.loss_add_tail,
        "max_completion_length": plan.max_completion_length,
        "num_generations": plan.num_generations,
        "generation_batch_size": plan.generation_batch_size,
        "per_device_train_batch_size": plan.per_device_train_batch_size,
        "gradient_accumulation_steps": plan.gradient_accumulation_steps,
        "use_vllm": plan.use_vllm_rollouts,
        "learning_rate": float(hp.get("learning_rate", 1e-5)),
        "num_train_epochs": float(hp.get(EPOCHS_HYPERPARAM, DEFAULT_EPOCHS)),
        "logging_steps": int(hp.get("logging_steps", 1)),
        "bf16": True,
        "report_to": [],
    }
    if plan.teacher_revision:
        kwargs["teacher_model_revision"] = plan.teacher_revision
    # TRL requires rollouts to come from the current weights for this objective,
    # and rejects any other sync frequency when vLLM generates them.
    if plan.objective == "iw_opd" and plan.use_vllm_rollouts:
        kwargs["vllm_sync_frequency"] = 1
    return kwargs
