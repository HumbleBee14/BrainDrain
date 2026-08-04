"""The report-only fidelity metric: mean forward KL(teacher ‖ student).

Two properties matter more than the arithmetic, which the loss's own tests
already pin. First, the metric must be the *same* quantity the run trained
against — so the value here is asserted equal to the loss's KD term rather than
to an independently derived number. Second, nothing on this path may raise into
an evaluation: a record it cannot score is skipped and counted.
"""

import json
import math
import shutil
from pathlib import Path

import pytest

# torch arrives with the ml extra, not the default worker install.
torch = pytest.importorskip("torch")

from src.activities.distill_loss import (  # noqa: E402
    IGNORE_INDEX,
    DistillLossConfig,
    distillation_loss,
    forward_kl_rows,
)
from src.activities.stubs import RunEvaluationInput  # noqa: E402
from src.activities.teacher_fidelity import (  # noqa: E402
    TeacherArtifacts,
    measure_distribution_match,
)
from src.teacher.artifacts import (  # noqa: E402
    PAD_LOGPROB,
    PAD_TOKEN_ID,
    ShardBuilder,
    build_manifest,
    write_shard,
)

VOCAB = 4
KD_ONLY = DistillLossConfig(kd_alpha=1.0, ce_alpha=0.0)

TOKENIZER_HASH = "tok-hash"
FINGERPRINT = "render-hash"


def _rows(*, support, tail_mass, width=None, pad=(PAD_TOKEN_ID, PAD_LOGPROB), student_probs):
    """One scored position: a student distribution against a teacher's support."""
    width = width or len(support)
    padding = width - len(support)
    return {
        "student_logits": torch.tensor(
            [[math.log(prob) for prob in student_probs]], dtype=torch.float32
        ),
        "teacher_token_ids": torch.tensor(
            [[token_id for token_id, _ in support] + [pad[0]] * padding], dtype=torch.long
        ),
        "teacher_logprobs": torch.tensor(
            [[math.log(prob) for _, prob in support] + [pad[1]] * padding], dtype=torch.float32
        ),
        "teacher_support_len": torch.tensor([len(support)], dtype=torch.long),
        "teacher_tail_mass": torch.tensor([tail_mass], dtype=torch.float32),
    }


def test_metric_is_the_same_quantity_the_loss_minimizes():
    """Teacher (0.5, 0.25, tail 0.25) vs student (0.4, 0.3, 0.2, 0.1).

    The loss reaches this through labels, chunking and a position shift; the
    metric reaches it directly. Both must land on the same number, or the report
    would describe an objective the run never had.
    """
    support = ((1, 0.5), (2, 0.25))
    student_probs = (0.4, 0.3, 0.2, 0.1)

    rows = forward_kl_rows(**_rows(support=support, tail_mass=0.25, student_probs=student_probs))

    logits = torch.zeros((1, 2, VOCAB), dtype=torch.float32)
    logits[0, 0] = torch.tensor([math.log(prob) for prob in student_probs])
    labels = torch.tensor([[IGNORE_INDEX, 1]], dtype=torch.long)
    token_ids = torch.zeros((1, 2, 2), dtype=torch.long)
    token_ids[0, 1] = torch.tensor([token_id for token_id, _ in support])
    logprobs = torch.zeros((1, 2, 2), dtype=torch.float32)
    logprobs[0, 1] = torch.tensor([math.log(prob) for _, prob in support])
    parts = distillation_loss(
        student_logits=logits,
        labels=labels,
        teacher_token_ids=token_ids,
        teacher_logprobs=logprobs,
        teacher_support_len=torch.tensor([[0, 2]], dtype=torch.long),
        teacher_tail_mass=torch.tensor([[0.0, 0.25]], dtype=torch.float32),
        config=KD_ONLY,
    )

    assert rows.item() == pytest.approx(0.13791190457156147, abs=1e-6)
    assert rows.item() == pytest.approx(parts.kd.item(), abs=1e-6)


@pytest.mark.parametrize("pad", [(PAD_TOKEN_ID, PAD_LOGPROB), (0, 0.0), (1, math.log(0.9))])
def test_padding_beyond_support_contributes_nothing(pad):
    kwargs = {
        "support": ((1, 0.5), (2, 0.25)),
        "tail_mass": 0.25,
        "student_probs": (0.4, 0.3, 0.2, 0.1),
    }

    tight = forward_kl_rows(**_rows(width=2, **kwargs))
    padded = forward_kl_rows(**_rows(width=4, pad=pad, **kwargs))

    assert padded.item() == tight.item()


def test_zero_tail_contributes_nothing_and_stays_finite():
    """Teacher (0.6, 0.4) with no tail vs student (0.3, 0.2, ...):

    0.6*ln(0.6/0.3) + 0.4*ln(0.4/0.2) = ln 2 = 0.6931472, and the missing tail
    must be gated to zero rather than reaching log(0).
    """
    rows = forward_kl_rows(
        **_rows(
            support=((1, 0.6), (2, 0.4)),
            tail_mass=0.0,
            student_probs=(0.4, 0.3, 0.2, 0.1),
        )
    )

    assert math.isfinite(rows.item())
    assert rows.item() == pytest.approx(math.log(2), abs=1e-6)


def test_a_student_matching_the_teacher_scores_zero():
    """Student mass equal to the teacher's, including 0.1 spread over the tail."""
    rows = forward_kl_rows(
        **_rows(
            support=((0, 0.5), (1, 0.25), (2, 0.15)),
            tail_mass=0.1,
            student_probs=(0.5, 0.25, 0.15, 0.1),
        )
    )

    assert rows.item() == pytest.approx(0.0, abs=1e-6)


class _MatchingLM(torch.nn.Module):
    """A student whose next-token distribution is exactly `probs`, everywhere.

    A real parameter is what makes this stand in for a loaded model: the metric
    reads the model's device off it.
    """

    def __init__(self, probs):
        super().__init__()
        self.logits = torch.nn.Parameter(torch.tensor([math.log(p) for p in probs]))

    def forward(self, input_ids=None, **_kwargs):
        rows = input_ids.shape[1]
        return type(
            "Output", (), {"logits": self.logits.expand(rows, len(self.logits))[None, :, :]}
        )()


def _shard(directory: Path, *, name: str, records=((4, 2),)) -> Path:
    """Records the teacher scored with (0.5, 0.25) plus a 0.25 tail everywhere."""
    builder = ShardBuilder(top_k=1)
    for index, (length, completion_start) in enumerate(records):
        builder.add_record(
            index,
            list(range(1, length + 1)),
            completion_start,
            [[(0, math.log(0.5)), (1, math.log(0.25))]] * (length - completion_start),
        )
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / name
    write_shard(str(path), builder.to_arrays())
    return path


def _artifacts(*paths: Path, records: int = 1) -> TeacherArtifacts:
    manifest = json.loads(
        json.dumps(
            build_manifest(
                top_k=1,
                teacher_model="Qwen/Qwen3-32B",
                teacher_revision="0" * 40,
                precision="bf16",
                tokenizer_hash=TOKENIZER_HASH,
                rendering_fingerprint=FINGERPRINT,
                vllm_version="0.26.0",
                max_batch_tokens=8192,
                created_at="2026-08-04T00:00:00Z",
                shards=[],
                skipped_records=0,
            )
        )
    )
    return TeacherArtifacts(manifest=manifest, shard_paths=tuple(paths))


def test_a_student_matching_the_stored_distributions_measures_zero(tmp_path):
    path = _shard(tmp_path, name="shard-00000.npz")
    model = _MatchingLM((0.5, 0.25, 0.15, 0.1))

    match = measure_distribution_match(model, _artifacts(path), max_seq_length=8)

    assert match is not None
    assert match.records == 1
    assert match.scored_positions == 2
    assert match.skipped_records == 0
    assert match.mean_kl == pytest.approx(0.0, abs=1e-5)


def test_a_diverging_student_measures_above_zero(tmp_path):
    path = _shard(tmp_path, name="shard-00000.npz")

    match = measure_distribution_match(
        _MatchingLM((0.1, 0.1, 0.4, 0.4)), _artifacts(path), max_seq_length=8
    )

    assert match is not None
    assert match.mean_kl > 0.1


def test_records_past_the_student_context_are_skipped_not_measured(tmp_path):
    path = _shard(tmp_path, name="shard-00000.npz", records=((4, 2), (6, 2)))

    match = measure_distribution_match(
        _MatchingLM((0.5, 0.25, 0.15, 0.1)), _artifacts(path), max_seq_length=4
    )

    assert match is not None
    assert match.records == 1
    assert match.skipped_records == 1
    assert match.scored_positions == 2


def test_nothing_scorable_reports_nothing(tmp_path):
    path = _shard(tmp_path, name="shard-00000.npz", records=((6, 2),))

    assert (
        measure_distribution_match(
            _MatchingLM((0.5, 0.25, 0.15, 0.1)), _artifacts(path), max_seq_length=4
        )
        is None
    )


def test_the_record_cap_bounds_the_forward_passes(tmp_path):
    first = _shard(tmp_path, name="a.npz")
    shutil.copyfile(first, tmp_path / "b.npz")

    match = measure_distribution_match(
        _MatchingLM((0.5, 0.25, 0.15, 0.1)),
        _artifacts(first, tmp_path / "b.npz"),
        max_seq_length=8,
        max_records=1,
    )

    assert match is not None
    assert match.records == 1


class _RecordingS3:
    """Serves objects from a dict of key -> bytes; unknown keys raise."""

    def __init__(self, objects):
        self.objects = objects
        self.keys = []

    def download_file(self, bucket, key, destination):
        self.keys.append(key)
        if key not in self.objects:
            raise KeyError(key)
        Path(destination).write_bytes(self.objects[key])


class _Hashes:
    combined_hash = TOKENIZER_HASH


def _patch_identity(monkeypatch, *, tokenizer_hash=TOKENIZER_HASH, fingerprint=FINGERPRINT):
    import src.teacher.rendering as rendering
    import src.teacher.tokenizer_identity as identity

    monkeypatch.setattr(
        identity,
        "compute_tokenizer_hashes",
        lambda model_id, hf_token="": type("H", (), {"combined_hash": tokenizer_hash})(),
    )
    monkeypatch.setattr(rendering, "rendering_fingerprint", lambda tokenizer: fingerprint)


def _eval_input(**overrides) -> RunEvaluationInput:
    fields = {
        "tenant_id": "t1",
        "model_id": "m1",
        "evaluation_id": "e1",
        "adapter_path": "adapters/t1/m1/",
        "base_model": "Qwen/Qwen3-8B",
        "dataset_path": "datasets/t1/p1/d1.jsonl",
        "mode": "distill",
        "job_config": {"teacher_artifacts_prefix": "datasets/t1/p1/d1-teacher-logprobs/abc/"},
    }
    fields.update(overrides)
    return RunEvaluationInput(**fields)


def _manifest_bytes(shards, **overrides) -> bytes:
    manifest = build_manifest(
        top_k=1,
        teacher_model="Qwen/Qwen3-32B",
        teacher_revision="0" * 40,
        precision="bf16",
        tokenizer_hash=TOKENIZER_HASH,
        rendering_fingerprint=FINGERPRINT,
        vllm_version="0.26.0",
        max_batch_tokens=8192,
        created_at="2026-08-04T00:00:00Z",
        shards=shards,
        skipped_records=0,
    )
    manifest.update(overrides)
    return json.dumps(manifest).encode("utf-8")


def test_artifacts_are_loaded_when_the_manifest_describes_this_student(monkeypatch, tmp_path):
    from src.activities.run_evaluation import _load_teacher_artifacts
    from src.teacher.artifacts import ShardEntry

    _patch_identity(monkeypatch)
    shard = _shard(tmp_path / "source", name="shard-00000.npz")
    prefix = "datasets/t1/p1/d1-teacher-logprobs/abc/"
    s3 = _RecordingS3(
        {
            f"{prefix}manifest.json": _manifest_bytes(
                [ShardEntry(name="shard-00000.npz", records=1, rows=2, first_record_index=0)]
            ),
            f"{prefix}shard-00000.npz": shard.read_bytes(),
        }
    )

    artifacts = _load_teacher_artifacts(
        _eval_input(),
        tmpdir=tmp_path,
        s3=s3,
        s3_bucket="bucket",
        tokenizer=object(),
        settings=None,
    )

    assert artifacts is not None
    assert artifacts.teacher_model == "Qwen/Qwen3-32B"
    assert len(artifacts.shard_paths) == 1


def test_a_hash_mismatch_is_skipped_not_raised(monkeypatch, tmp_path):
    from src.activities.run_evaluation import _load_teacher_artifacts
    from src.teacher.artifacts import ShardEntry

    _patch_identity(monkeypatch, tokenizer_hash="a-different-student")
    prefix = "datasets/t1/p1/d1-teacher-logprobs/abc/"
    s3 = _RecordingS3(
        {
            f"{prefix}manifest.json": _manifest_bytes(
                [ShardEntry(name="shard-00000.npz", records=1, rows=2, first_record_index=0)]
            )
        }
    )

    assert (
        _load_teacher_artifacts(
            _eval_input(),
            tmpdir=tmp_path,
            s3=s3,
            s3_bucket="bucket",
            tokenizer=object(),
            settings=None,
        )
        is None
    )
    assert s3.keys == [f"{prefix}manifest.json"]


def test_missing_artifacts_are_skipped_not_raised(monkeypatch, tmp_path):
    from src.activities.run_evaluation import _load_teacher_artifacts

    _patch_identity(monkeypatch)

    assert (
        _load_teacher_artifacts(
            _eval_input(),
            tmpdir=tmp_path,
            s3=_RecordingS3({}),
            s3_bucket="bucket",
            tokenizer=object(),
            settings=None,
        )
        is None
    )


@pytest.mark.parametrize(
    "overrides",
    [
        {"mode": "quick"},
        {"mode": ""},
        {"job_config": {}},
        {"job_config": {"teacher_artifacts_prefix": ""}},
    ],
)
def test_runs_without_stored_distributions_look_for_nothing(monkeypatch, tmp_path, overrides):
    """No S3 call at all — the absence is decided from the job, not from a miss."""
    from src.activities.run_evaluation import _load_teacher_artifacts

    _patch_identity(monkeypatch)
    s3 = _RecordingS3({})

    assert (
        _load_teacher_artifacts(
            _eval_input(**overrides),
            tmpdir=tmp_path,
            s3=s3,
            s3_bucket="bucket",
            tokenizer=object(),
            settings=None,
        )
        is None
    )
    assert s3.keys == []
