"""Loss for offline logit distillation against a teacher's stored top-k.

The teacher scored the data once and left, so training never sees its logits —
only the top-k support, matching logprobs, and the probability mass that fell
outside that support (`tail_mass`) per scored position. That shape decides the
maths: forward KL is computed over the partition of the vocabulary the artifact
actually describes — one bucket per stored token plus one merged bucket for
everything else — which is exact for the student side (its probabilities over
that partition sum to 1 by construction) and coarse only for the teacher's tail.

Two departures from the closest published reference are deliberate:

**The tail is modelled, not discarded.** The usual offline-KD implementation
renormalizes the teacher over its top-k, which quietly asserts the teacher had no
opinion outside it. Here the tail is a term of its own, so a teacher that spread
30% of its mass thinly still pushes the student to keep mass there.

**Forward KL, not reverse.** Correct for this off-policy, teacher-forced setting:
the targets were computed on the teacher's own text, not the student's rollouts.

Nothing here imports a training framework — it takes tensors and returns tensors,
so every rule below (padding contributes nothing, prompt positions contribute
nothing, a matching student scores zero) is checked on the CPU without a GPU.
"""

from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    import torch

# HuggingFace's convention for "not supervised here"; the only authority on which
# positions the loss counts.
IGNORE_INDEX = -100

DEFAULT_KD_ALPHA = 0.9
DEFAULT_CE_ALPHA = 0.1
DEFAULT_TEMPERATURE = 1.0

# Published work reports that amplifying the tail term (beta around 2) beats both
# discarding the tail and folding it into the top-k. That constant was tuned on
# someone else's data, and shipping it as our default would assert a result we
# have not measured, so the default is the unamplified case and the knob exists
# for the parity harness to earn a change.
DEFAULT_TAIL_BETA = 1.0

DEFAULT_CHUNK_ROWS = 512

# Stand-in for -inf on masked entries. Every intermediate stays finite, so a
# padded entry contributes 0 * -1e30 == -0.0 rather than 0 * -inf == NaN.
_NEG_FILL = -1.0e30

# Floor on log(student mass outside the support). The complement is obtained by
# logsumexp so it can never be -inf, but a student that has put essentially all
# its mass inside the support drives the tail term and its gradient arbitrarily
# far on what is fp32 noise.
_MIN_LOG_OUTSIDE = -30.0


@dataclass(frozen=True)
class DistillLossConfig:
    """Mixing weights for the two supervision signals.

    The defaults match the only real-world offline-KD implementation's documented
    defaults: the teacher's distribution carries the run, hard labels stay in as a
    small anchor to the tokens actually written.
    """

    kd_alpha: float = DEFAULT_KD_ALPHA
    ce_alpha: float = DEFAULT_CE_ALPHA
    temperature: float = DEFAULT_TEMPERATURE
    tail_beta: float = DEFAULT_TAIL_BETA
    chunk_rows: int = DEFAULT_CHUNK_ROWS

    def __post_init__(self) -> None:
        if self.temperature <= 0.0:
            raise ValueError(f"temperature must be positive, got {self.temperature}")
        if self.kd_alpha < 0.0 or self.ce_alpha < 0.0:
            raise ValueError("kd_alpha and ce_alpha must not be negative")
        if self.kd_alpha == 0.0 and self.ce_alpha == 0.0:
            raise ValueError("kd_alpha and ce_alpha cannot both be zero: nothing would be learned")
        if self.tail_beta < 0.0:
            raise ValueError(f"tail_beta must not be negative, got {self.tail_beta}")
        if self.chunk_rows < 1:
            raise ValueError(f"chunk_rows must be at least 1, got {self.chunk_rows}")

    @classmethod
    def from_hyperparams(cls, hp: dict) -> "DistillLossConfig":
        return cls(
            kd_alpha=float(hp.get("kd_alpha", DEFAULT_KD_ALPHA)),
            ce_alpha=float(hp.get("ce_alpha", DEFAULT_CE_ALPHA)),
            temperature=float(hp.get("kd_temperature", DEFAULT_TEMPERATURE)),
            tail_beta=float(hp.get("tail_beta", DEFAULT_TAIL_BETA)),
            chunk_rows=int(hp.get("kd_chunk_rows", DEFAULT_CHUNK_ROWS)),
        )


@dataclass(frozen=True)
class DistillLossParts:
    """The mixed loss plus its components, for logging what actually moved."""

    loss: "torch.Tensor"
    kd: "torch.Tensor"
    ce: "torch.Tensor"
    supervised_tokens: int


def distillation_loss(
    *,
    student_logits: "torch.Tensor",
    labels: "torch.Tensor",
    teacher_token_ids: "torch.Tensor",
    teacher_logprobs: "torch.Tensor",
    teacher_support_len: "torch.Tensor",
    teacher_tail_mass: "torch.Tensor",
    config: DistillLossConfig | None = None,
) -> DistillLossParts:
    """`kd_alpha * forward_KL(teacher || student) + ce_alpha * CE(hard labels)`.

    `student_logits` is `[batch, seq, vocab]` straight from a forward pass over
    the artifact's own token ids. Teacher arrays live on the same `[batch, seq]`
    grid, entry `t` describing the distribution over the token *at* `t`; the
    student's prediction for it comes from the logits at `t - 1`, and that shift
    is applied here so exactly one place in the codebase owns it.

    Both means are taken over the number of supervised tokens in the batch, so
    batches of different completion lengths are directly comparable.
    """
    import torch

    config = config or DistillLossConfig()
    teacher = {
        "teacher_token_ids": teacher_token_ids,
        "teacher_logprobs": teacher_logprobs,
        "teacher_support_len": teacher_support_len,
        "teacher_tail_mass": teacher_tail_mass,
    }
    _check_shapes(student_logits, labels, teacher)
    _check_supervision(labels, teacher_support_len)

    batch, seq, vocab = student_logits.shape
    width = teacher_token_ids.shape[-1]
    flat_logits = student_logits.reshape(batch * seq, vocab)
    flat_labels = labels.reshape(-1)
    flat_ids = teacher_token_ids.reshape(batch * seq, width)
    flat_logprobs = teacher_logprobs.reshape(batch * seq, width)
    flat_support = teacher_support_len.reshape(-1)
    flat_tail = teacher_tail_mass.reshape(-1)

    positions = (flat_labels != IGNORE_INDEX).nonzero(as_tuple=True)[0]
    supervised_tokens = int(positions.numel())
    if supervised_tokens == 0:
        raise ValueError("batch has no supervised positions")

    lane = torch.arange(width, device=student_logits.device)
    kd_sum = student_logits.new_zeros((), dtype=torch.float32)
    ce_sum = student_logits.new_zeros((), dtype=torch.float32)

    for start in range(0, supervised_tokens, config.chunk_rows):
        rows = positions[start : start + config.chunk_rows]
        logits = flat_logits.index_select(0, rows - 1).float()
        support = flat_support.index_select(0, rows).long()
        valid = lane[None, :] < support[:, None]
        ids = flat_ids.index_select(0, rows).long()
        # Padded lanes are pointed at a real support id so scattering them is
        # idempotent; `valid` is what keeps them out of the sums.
        ids = torch.where(valid, ids, ids[:, :1])
        tempered_logprobs = torch.log_softmax(logits / config.temperature, dim=-1)

        kd_rows = _kd_rows(
            tempered_logprobs=tempered_logprobs,
            ids=ids,
            valid=valid,
            teacher_logprobs=flat_logprobs.index_select(0, rows).float(),
            tail_mass=flat_tail.index_select(0, rows).float(),
            config=config,
        )
        ce_rows = _ce_rows(
            logits=logits,
            tempered_logprobs=tempered_logprobs,
            labels=flat_labels.index_select(0, rows).long(),
            temperature=config.temperature,
        )
        kd_sum = kd_sum + kd_rows.sum()
        ce_sum = ce_sum + ce_rows.sum()

    kd = kd_sum / supervised_tokens
    ce = ce_sum / supervised_tokens
    return DistillLossParts(
        loss=config.kd_alpha * kd + config.ce_alpha * ce,
        kd=kd,
        ce=ce,
        supervised_tokens=supervised_tokens,
    )


def _kd_rows(
    *,
    tempered_logprobs: "torch.Tensor",
    ids: "torch.Tensor",
    valid: "torch.Tensor",
    teacher_logprobs: "torch.Tensor",
    tail_mass: "torch.Tensor",
    config: DistillLossConfig,
) -> "torch.Tensor":
    """Per-position forward KL over {each stored token} + {everything else}.

    The teacher is renormalized across that partition after tempering. At T = 1
    this is a no-op up to float noise, because `tail_mass` is defined as the
    remainder of the stored support; at any other temperature it is what makes
    "apply the temperature to both sides" well defined for a distribution we only
    hold sparsely.
    """
    import torch

    temperature = config.temperature
    masked_out = torch.full_like(teacher_logprobs, _NEG_FILL)
    tempered_teacher = torch.where(valid, teacher_logprobs / temperature, masked_out)

    positive_tail = tail_mass > 0
    safe_tail = torch.where(positive_tail, tail_mass, torch.ones_like(tail_mass))
    tail_logprob = torch.where(
        positive_tail,
        torch.log(safe_tail) / temperature,
        torch.full_like(tail_mass, _NEG_FILL),
    )

    partition = torch.cat([tempered_teacher, tail_logprob[:, None]], dim=1)
    teacher_log = torch.log_softmax(partition, dim=1)
    teacher_prob = teacher_log.exp()

    support_logprobs = torch.gather(tempered_logprobs, 1, ids)
    support_terms = teacher_prob[:, :-1] * (teacher_log[:, :-1] - support_logprobs)
    top_k = torch.where(valid, support_terms, torch.zeros_like(support_terms)).sum(dim=1)

    # Mass outside the support via the complement rather than 1 - sum(exp(...)):
    # subtracting from one loses all precision exactly where a confident student
    # makes the tail term matter most.
    outside_logprob = tempered_logprobs.scatter(1, ids, _NEG_FILL).logsumexp(dim=1)
    outside_logprob = outside_logprob.clamp(min=_MIN_LOG_OUTSIDE)
    tail_term = teacher_prob[:, -1] * (teacher_log[:, -1] - outside_logprob)
    tail = torch.where(positive_tail, tail_term, torch.zeros_like(tail_term))

    # Hinton's T^2: tempering both sides scales the KD gradient by 1/T^2, so this
    # keeps it comparable to the untempered CE term as T changes. It applies to
    # the KD term only, and is exactly 1 at the default T = 1.
    return (top_k + config.tail_beta * tail) * (temperature * temperature)


def _ce_rows(
    *,
    logits: "torch.Tensor",
    tempered_logprobs: "torch.Tensor",
    labels: "torch.Tensor",
    temperature: float,
) -> "torch.Tensor":
    """Hard-label cross-entropy, always at T = 1 — only the KD term is tempered."""
    import torch

    if temperature == 1.0:
        return -torch.gather(tempered_logprobs, 1, labels[:, None]).squeeze(1)
    return torch.nn.functional.cross_entropy(logits, labels, reduction="none")


def collate_distill_batch(
    records: list[dict[str, Any]], *, pad_token_id: int
) -> dict[str, "torch.Tensor"]:
    """Pad `artifacts.record_view` outputs into the tensors the loss expects.

    Right-padded. Padding positions carry `IGNORE_INDEX` labels and zero support,
    so they are invisible to both the attention mask and the loss.
    """
    import numpy as np
    import torch

    if not records:
        raise ValueError("cannot collate an empty batch")

    lengths = [len(record["input_ids"]) for record in records]
    rows = len(records)
    seq = max(lengths)
    width = int(records[0]["token_ids"].shape[1])

    input_ids = np.full((rows, seq), pad_token_id, dtype=np.int64)
    attention_mask = np.zeros((rows, seq), dtype=np.int64)
    labels = np.full((rows, seq), IGNORE_INDEX, dtype=np.int64)
    teacher_token_ids = np.zeros((rows, seq, width), dtype=np.int64)
    teacher_logprobs = np.zeros((rows, seq, width), dtype=np.float32)
    teacher_support_len = np.zeros((rows, seq), dtype=np.int64)
    teacher_tail_mass = np.zeros((rows, seq), dtype=np.float32)

    for row, record in enumerate(records):
        length = lengths[row]
        start = int(record["completion_start"])
        if start < 1:
            raise ValueError(
                "a record supervised from position 0 has no context for its first "
                "target, so the teacher could not have scored it"
            )
        scored = record["token_ids"].shape
        if scored != (length - start, width):
            raise ValueError(
                f"record has {scored} scored distributions for a supervised span of "
                f"{length - start} positions at width {width}"
            )
        ids = np.asarray(record["input_ids"], dtype=np.int64)
        input_ids[row, :length] = ids
        attention_mask[row, :length] = 1
        labels[row, start:length] = ids[start:length]
        teacher_token_ids[row, start:length] = record["token_ids"]
        teacher_logprobs[row, start:length] = record["logprobs"]
        teacher_support_len[row, start:length] = record["support_len"]
        teacher_tail_mass[row, start:length] = record["tail_mass"]

    return {
        "input_ids": torch.from_numpy(input_ids),
        "attention_mask": torch.from_numpy(attention_mask),
        "labels": torch.from_numpy(labels),
        "teacher_token_ids": torch.from_numpy(teacher_token_ids),
        "teacher_logprobs": torch.from_numpy(teacher_logprobs),
        "teacher_support_len": torch.from_numpy(teacher_support_len),
        "teacher_tail_mass": torch.from_numpy(teacher_tail_mass),
    }


def _check_shapes(
    student_logits: "torch.Tensor",
    labels: "torch.Tensor",
    teacher: dict[str, "torch.Tensor"],
) -> None:
    if student_logits.dim() != 3:
        raise ValueError(
            f"student_logits must be [batch, seq, vocab], got {tuple(student_logits.shape)}"
        )
    if teacher["teacher_token_ids"].dim() != 3:
        raise ValueError(
            "teacher_token_ids must be [batch, seq, width], got "
            f"{tuple(teacher['teacher_token_ids'].shape)}"
        )

    batch, seq, _ = student_logits.shape
    width = teacher["teacher_token_ids"].shape[-1]
    expected = {
        "labels": (labels, (batch, seq)),
        "teacher_token_ids": (teacher["teacher_token_ids"], (batch, seq, width)),
        "teacher_logprobs": (teacher["teacher_logprobs"], (batch, seq, width)),
        "teacher_support_len": (teacher["teacher_support_len"], (batch, seq)),
        "teacher_tail_mass": (teacher["teacher_tail_mass"], (batch, seq)),
    }
    for name, (tensor, shape) in expected.items():
        if tuple(tensor.shape) != shape:
            raise ValueError(f"{name} has shape {tuple(tensor.shape)}, expected {shape}")


def _check_supervision(labels: "torch.Tensor", teacher_support_len: "torch.Tensor") -> None:
    """Refuse a batch whose labels and teacher support describe different positions.

    Both errors are silent otherwise: the run trains on shifted or absent targets
    and its loss curve looks healthy the whole way down.
    """
    supervised = labels != IGNORE_INDEX
    if bool(supervised[:, 0].any()):
        raise ValueError(
            "position 0 cannot be supervised: nothing precedes it, so no teacher "
            "distribution exists for it"
        )
    if bool((supervised != (teacher_support_len > 0)).any()):
        raise ValueError(
            "supervised labels and teacher support disagree; the batch and the "
            "artifact are misaligned"
        )
