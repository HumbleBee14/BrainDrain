"""Dispatch and admission for the logit-distillation strategy.

Two things are worth locking down here. The registry key is composite
(`mode` + `distill_method`) so the public `TrainingMode` never learns the internal
strategy name, and a manifest that disagrees with this student's tokenization is
refused before any GPU time is spent on targets that would be shifted.
"""

import math
import shutil
from pathlib import Path

import pytest
from temporalio.exceptions import ApplicationError

from src.activities.train_model import (
    _TEACHER_ARTIFACT_MISMATCH,
    DistillLogitStrategy,
    _truncate_record_view,
    _verify_teacher_manifest,
    resolve_strategy_key,
)
from src.activities.training_engine import get_strategy

REPO_ROOT = Path(__file__).resolve().parents[3]

TOKENIZER_HASH = "aaaa1111"
FINGERPRINT = "bbbb2222"


@pytest.mark.parametrize(
    ("mode", "method", "expected"),
    [
        ("distill", None, "distill"),
        ("distill", "", "distill"),
        ("distill", "text", "distill"),
        ("distill", "logit", "distill_logit"),
        ("quick", None, "quick"),
        ("quick", "text", "quick"),
        ("aligned", None, "aligned"),
        ("reasoning", None, "reasoning"),
        ("iterative", None, "iterative"),
    ],
)
def test_strategy_key_mapping(mode, method, expected):
    assert resolve_strategy_key(mode, method) == expected


@pytest.mark.parametrize("method", ["logit", "bogus"])
def test_a_method_outside_distill_mode_is_refused(method):
    """A fidelity request on a non-distill mode means the job is misconfigured.

    Ignoring it would train plain SFT and report success for a run whose teacher
    artifacts were already paid for.
    """
    with pytest.raises(ValueError, match="no meaning for mode"):
        resolve_strategy_key("quick", method)


def test_unknown_distill_method_is_refused():
    with pytest.raises(ValueError, match="Unknown distill_method"):
        resolve_strategy_key("distill", "logits")


def test_unknown_mode_passes_through_to_the_registry():
    # get_strategy owns the "unknown mode" error message; resolution does not
    # duplicate it.
    assert resolve_strategy_key("whatever", None) == "whatever"
    with pytest.raises(ValueError, match="Unknown training mode"):
        get_strategy(resolve_strategy_key("whatever", None))


@pytest.mark.parametrize("mode", ["quick", "distill", "aligned", "reasoning"])
def test_every_resolved_key_is_registered(mode):
    assert get_strategy(resolve_strategy_key(mode, None)) is not None


def test_logit_strategy_is_registered_under_the_composite_key():
    strategy = get_strategy(resolve_strategy_key("distill", "logit"))

    assert isinstance(strategy, DistillLogitStrategy)
    assert strategy.name == "distill_logit"


def test_the_internal_strategy_name_never_reaches_the_public_enum():
    for relative in (
        "crates/shared/src/enums.rs",
        "apps/web/src/lib/generated/TrainingMode.ts",
    ):
        assert "distill_logit" not in (REPO_ROOT / relative).read_text()


def _manifest(*, tokenizer_hash=TOKENIZER_HASH, rendering_fingerprint=FINGERPRINT):
    return {"tokenizer_hash": tokenizer_hash, "rendering_fingerprint": rendering_fingerprint}


@pytest.fixture
def student(monkeypatch):
    """Pin what this student's own tokenizer and template hash to."""
    import src.teacher.rendering as rendering
    import src.teacher.tokenizer_identity as tokenizer_identity

    class _Hashes:
        combined_hash = TOKENIZER_HASH

    monkeypatch.setattr(tokenizer_identity, "compute_tokenizer_hashes", lambda *a, **k: _Hashes())
    monkeypatch.setattr(rendering, "rendering_fingerprint", lambda tokenizer: FINGERPRINT)
    return object()


def test_matching_manifest_is_accepted(student):
    _verify_teacher_manifest(_manifest(), base_model="org/student", tokenizer=student)


@pytest.mark.parametrize(
    "manifest",
    [
        _manifest(tokenizer_hash="different"),
        _manifest(rendering_fingerprint="different"),
        {},
    ],
)
def test_disagreeing_manifest_is_refused(student, manifest):
    with pytest.raises(ApplicationError) as excinfo:
        _verify_teacher_manifest(manifest, base_model="org/student", tokenizer=student)

    assert excinfo.value.non_retryable is True
    assert str(excinfo.value) == _TEACHER_ARTIFACT_MISMATCH


def _view(*, length, completion_start):
    np = pytest.importorskip("numpy")
    scored = length - completion_start
    return {
        "input_ids": np.arange(length, dtype=np.uint32),
        "completion_start": completion_start,
        "token_ids": np.zeros((scored, 3), dtype=np.uint32),
        "logprobs": np.zeros((scored, 3), dtype=np.float16),
        "support_len": np.full(scored, 3, dtype=np.uint16),
        "tail_mass": np.zeros(scored, dtype=np.float16),
    }


def test_a_record_within_the_context_is_untouched():
    view = _view(length=10, completion_start=4)

    assert _truncate_record_view(view, 10) is view


def test_truncation_keeps_the_supervised_span_aligned():
    truncated = _truncate_record_view(_view(length=10, completion_start=4), 8)

    assert len(truncated["input_ids"]) == 8
    assert truncated["completion_start"] == 4
    assert truncated["token_ids"].shape[0] == 4
    assert truncated["support_len"].shape[0] == 4


def test_a_record_whose_prompt_fills_the_context_is_dropped():
    assert _truncate_record_view(_view(length=10, completion_start=6), 6) is None


class _FakeS3:
    """Serves shards from a local directory and records what was asked for."""

    def __init__(self, source):
        self.source = source
        self.keys = []

    def download_file(self, bucket, key, destination):
        self.keys.append(key)
        shutil.copyfile(self.source / Path(key).name, destination)


def test_a_streamed_shard_reaches_the_loss_through_its_stored_token_ids(tmp_path):
    """The whole data path minus the Trainer: shard on S3 -> batch -> loss.

    The record's own token ids are what the student is trained on, so this also
    pins that training never re-tokenizes the dataset behind the artifact's back.
    """
    torch = pytest.importorskip("torch")
    from src.activities.distill_loss import collate_distill_batch, distillation_loss
    from src.activities.train_model import _build_artifact_dataset_class
    from src.teacher.artifacts import ShardBuilder, write_shard

    builder = ShardBuilder(top_k=2)
    builder.add_record(
        0,
        [5, 6, 7, 8],
        2,
        [
            [(7, math.log(0.6)), (9, math.log(0.3))],
            [(8, math.log(0.5)), (9, math.log(0.25))],
        ],
    )
    write_shard(str(tmp_path / "shard-0.npz"), builder.to_arrays())

    work = tmp_path / "work"
    work.mkdir()
    s3 = _FakeS3(tmp_path)
    dataset = _build_artifact_dataset_class()(
        manifest={"shards": [{"name": "shard-0.npz", "records": 1, "rows": 2}]},
        prefix="artifacts/tenant/dataset/",
        tmpdir=work,
        s3=s3,
        bucket="bucket",
        passes=2,
        max_seq_length=8,
    )

    records = list(dataset)

    assert s3.keys == ["artifacts/tenant/dataset/shard-0.npz"] * 2
    assert [int(token) for token in records[0]["input_ids"]] == [5, 6, 7, 8]
    assert list(work.iterdir()) == []

    batch = collate_distill_batch(records[:1], pad_token_id=0)
    parts = distillation_loss(
        student_logits=torch.zeros((1, 4, 16), dtype=torch.float32),
        labels=batch["labels"],
        teacher_token_ids=batch["teacher_token_ids"],
        teacher_logprobs=batch["teacher_logprobs"],
        teacher_support_len=batch["teacher_support_len"],
        teacher_tail_mass=batch["teacher_tail_mass"],
    )

    assert parts.supervised_tokens == 2
    assert math.isfinite(parts.loss.item())
