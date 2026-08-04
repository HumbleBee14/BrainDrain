"""How far a distilled student's distributions sit from its teacher's.

Report-only measurement for the parity suite. The teacher's scored distributions
are still on disk from the training run, so the student can be scored against the
exact quantity it was trained to minimize — the mean forward KL of
`distill_loss`, recomputed through that same code rather than a second
implementation of the formula.

Nothing here is allowed to matter enough to fail an evaluation: a record the
student cannot be run over is skipped and counted, and a measurement with no
usable position at all returns None so the suite reports nothing.
"""

from dataclasses import dataclass
from pathlib import Path

DEFAULT_MAX_RECORDS = 32


@dataclass(frozen=True)
class TeacherArtifacts:
    """A teacher's scored distributions, fetched and verified against a student."""

    manifest: dict
    shard_paths: tuple[Path, ...]

    @property
    def teacher_model(self) -> str:
        teacher = self.manifest.get("teacher")
        if not isinstance(teacher, dict):
            return ""
        return str(teacher.get("model", ""))


@dataclass(frozen=True)
class DistributionMatch:
    """Mean per-token forward KL, and how much data it was measured over."""

    mean_kl: float
    scored_positions: int
    records: int
    skipped_records: int


def measure_distribution_match(
    model,
    artifacts: TeacherArtifacts,
    *,
    max_seq_length: int,
    max_records: int = DEFAULT_MAX_RECORDS,
) -> DistributionMatch | None:
    """Mean forward KL(teacher ‖ student) over the teacher's stored positions.

    Averaged per scored token, not per record, so records of different completion
    lengths do not weight the result by how long they happen to be.
    """
    from src.teacher.artifacts import read_shard, record_view

    total_kl = 0.0
    positions = 0
    records = 0
    skipped = 0

    for path in artifacts.shard_paths:
        if records >= max_records:
            break
        arrays = read_shard(str(path))
        for position in range(int(arrays["record_index"].size)):
            if records >= max_records:
                break
            view = record_view(arrays, position)
            # A record longer than the student's context cannot be scored as the
            # teacher saw it, and one supervised from position 0 has no logits
            # predicting its first target.
            if len(view["input_ids"]) > max_seq_length or int(view["completion_start"]) < 1:
                skipped += 1
                continue
            row_kl = _record_kl(model, view)
            total_kl += float(row_kl.sum())
            positions += int(row_kl.numel())
            records += 1

    if positions == 0:
        return None
    return DistributionMatch(
        mean_kl=total_kl / positions,
        scored_positions=positions,
        records=records,
        skipped_records=skipped,
    )


def _record_kl(model, view: dict):
    """Per-position KL for one scored record, on the student's own device."""
    import numpy as np
    import torch

    from src.activities.distill_loss import forward_kl_rows

    device = next(model.parameters()).device

    def as_tensor(name: str, dtype) -> "torch.Tensor":
        return torch.from_numpy(np.asarray(view[name]).astype(dtype)).to(device)

    with torch.no_grad():
        input_ids = as_tensor("input_ids", np.int64)[None, :]
        logits = model(input_ids=input_ids).logits[0]

        # The teacher's row for position t is predicted by the logits at t - 1,
        # and its first scored position is `completion_start`.
        start = int(view["completion_start"])
        return forward_kl_rows(
            student_logits=logits[start - 1 : input_ids.shape[1] - 1],
            teacher_token_ids=as_tensor("token_ids", np.int64),
            teacher_logprobs=as_tensor("logprobs", np.float32),
            teacher_support_len=as_tensor("support_len", np.int64),
            teacher_tail_mass=as_tensor("tail_mass", np.float32),
        ).cpu()
