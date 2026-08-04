"""Correctness of the offline logit-distillation loss, against numbers worked by hand.

Every expected value below is derived in the test's own docstring from a four-token
vocabulary, so a refactor that changes the maths fails here rather than producing a
quietly worse model. The reference student distribution throughout is
`[0.4, 0.3, 0.2, 0.1]`, supplied as logits equal to its logs — a distribution that
already sums to one is its own log-softmax, which keeps the arithmetic exact.
"""

import math

import pytest

# torch arrives with the ml extra, not the default worker install; these tests are
# pure CPU tensor maths and skip rather than error where it is absent.
torch = pytest.importorskip("torch")

from src.activities.distill_loss import (  # noqa: E402
    IGNORE_INDEX,
    DistillLossConfig,
    collate_distill_batch,
    distillation_loss,
)
from src.teacher.artifacts import PAD_LOGPROB, PAD_TOKEN_ID  # noqa: E402

VOCAB = 4
STUDENT_PROBS = (0.4, 0.3, 0.2, 0.1)

# Teacher: half its mass on token 1, a quarter on token 2, a quarter outside.
SUPPORT = ((1, 0.5), (2, 0.25))
TAIL_MASS = 0.25
LABEL = 1

KD_ONLY = DistillLossConfig(kd_alpha=1.0, ce_alpha=0.0)
CE_ONLY = DistillLossConfig(kd_alpha=0.0, ce_alpha=1.0)


def _batch(
    *,
    support=SUPPORT,
    tail_mass=TAIL_MASS,
    label=LABEL,
    width=None,
    pad=(PAD_TOKEN_ID, PAD_LOGPROB),
    prompt_len=1,
    seq_len=None,
    student_probs=STUDENT_PROBS,
    other_logit=0.0,
):
    """One supervised position at index `prompt_len`; everything else is prompt.

    The student distribution is placed at the logits index that predicts that
    position (`prompt_len - 1`); every other index gets `other_logit`, which the
    masking test varies.
    """
    width = width or len(support)
    seq_len = seq_len or prompt_len + 1

    logits = torch.full((1, seq_len, VOCAB), other_logit, dtype=torch.float32)
    logits[0, prompt_len - 1] = torch.tensor([math.log(p) for p in student_probs])

    labels = torch.full((1, seq_len), IGNORE_INDEX, dtype=torch.long)
    labels[0, prompt_len] = label

    token_ids = torch.zeros((1, seq_len, width), dtype=torch.long)
    logprobs = torch.zeros((1, seq_len, width), dtype=torch.float32)
    support_len = torch.zeros((1, seq_len), dtype=torch.long)
    tail = torch.zeros((1, seq_len), dtype=torch.float32)

    padding = width - len(support)
    token_ids[0, prompt_len] = torch.tensor(
        [token_id for token_id, _ in support] + [pad[0]] * padding
    )
    logprobs[0, prompt_len] = torch.tensor(
        [math.log(prob) for _, prob in support] + [pad[1]] * padding
    )
    support_len[0, prompt_len] = len(support)
    tail[0, prompt_len] = tail_mass

    return {
        "student_logits": logits,
        "labels": labels,
        "teacher_token_ids": token_ids,
        "teacher_logprobs": logprobs,
        "teacher_support_len": support_len,
        "teacher_tail_mass": tail,
    }


def test_hand_computed_forward_kl():
    """KD is the forward KL over {token 1}, {token 2}, {everything else}.

    Teacher (0.5, 0.25, 0.25) against student (0.3, 0.2, 0.5), where the student's
    outside mass is 1 - 0.3 - 0.2 = 0.5:

        top-k = 0.5*ln(0.5/0.3) + 0.25*ln(0.25/0.2) = 0.2554128 + 0.0557859
        tail  = 0.25*ln(0.25/0.5)                   = -0.1732868
        KD    = 0.3111987 - 0.1732868              =  0.1379119
    """
    parts = distillation_loss(**_batch(), config=KD_ONLY)

    assert parts.kd.item() == pytest.approx(0.13791190457156147, abs=1e-6)
    assert parts.loss.item() == pytest.approx(parts.kd.item(), abs=1e-9)
    assert parts.supervised_tokens == 1


def test_default_alpha_mix():
    """0.9 * 0.1379119 + 0.1 * CE, with CE = -ln(0.3) = 1.2039728 -> 0.2445180."""
    parts = distillation_loss(**_batch())

    assert parts.ce.item() == pytest.approx(1.2039728043259361, abs=1e-6)
    assert parts.loss.item() == pytest.approx(0.24451799454699896, abs=1e-6)


def test_alpha_mixing_reduces_to_single_terms():
    kd_only = distillation_loss(**_batch(), config=KD_ONLY)
    ce_only = distillation_loss(**_batch(), config=CE_ONLY)

    assert kd_only.loss.item() == pytest.approx(0.13791190457156147, abs=1e-6)
    assert ce_only.loss.item() == pytest.approx(1.2039728043259361, abs=1e-6)


@pytest.mark.parametrize("pad", [(PAD_TOKEN_ID, PAD_LOGPROB), (0, 0.0), (1, math.log(0.9))])
def test_padding_beyond_support_len_contributes_nothing(pad):
    """A four-wide row with two real entries scores exactly as a two-wide row.

    The hostile cases matter more than the declared padding: `(0, 0.0)` claims
    probability 1 on a token that holds student mass, and `(1, ln 0.9)` duplicates
    a real support id. Both must be ignored on the authority of `support_len`.
    """
    tight = distillation_loss(**_batch(width=2), config=KD_ONLY)
    padded = distillation_loss(**_batch(width=4, pad=pad), config=KD_ONLY)

    assert padded.kd.item() == tight.kd.item()


def test_zero_tail_mass_contributes_nothing_and_stays_finite():
    """Teacher (0.6, 0.4) with no tail against student (0.3, 0.2):

    0.6*ln(0.6/0.3) + 0.4*ln(0.4/0.2) = (0.6 + 0.4)*ln 2 = 0.6931472,
    and log(0) never appears.
    """
    batch = _batch(support=((1, 0.6), (2, 0.4)), tail_mass=0.0, width=4)
    batch["student_logits"].requires_grad_(True)
    parts = distillation_loss(**batch, config=KD_ONLY)

    assert math.isfinite(parts.kd.item())
    assert parts.kd.item() == pytest.approx(math.log(2.0), abs=1e-6)

    parts.loss.backward()
    assert torch.isfinite(batch["student_logits"].grad).all()


def test_prompt_positions_do_not_affect_the_loss():
    """Only the logits index that predicts a supervised position may matter."""
    baseline = distillation_loss(**_batch(prompt_len=2, seq_len=4))

    perturbed = _batch(prompt_len=2, seq_len=4)
    for index in (0, 2, 3):
        perturbed["student_logits"][0, index] = torch.tensor([9.0, -9.0, 4.0, -4.0])
    after = distillation_loss(**perturbed)

    assert after.loss.item() == baseline.loss.item()


def test_matching_student_has_zero_kd():
    """Student (0.125, 0.5, 0.25, 0.125) reproduces the teacher exactly."""
    parts = distillation_loss(**_batch(student_probs=(0.125, 0.5, 0.25, 0.125)), config=KD_ONLY)

    assert parts.kd.item() == pytest.approx(0.0, abs=1e-6)


def test_temperature_is_applied_to_both_sides_and_scaled_by_t_squared():
    """At T = 2 both distributions are tempered and renormalized.

    Student becomes (0.325401, 0.281806, 0.230093, 0.162700) and the teacher's
    partition becomes (0.414214, 0.292893, 0.292893), giving

        top-k = 0.2302225, tail = -0.1495850, KD = 4 * 0.0806376 = 0.3225503.

    CE is untempered by convention, so it is unchanged at -ln(0.3).
    """
    tempered = distillation_loss(
        **_batch(), config=DistillLossConfig(kd_alpha=1.0, ce_alpha=1.0, temperature=2.0)
    )

    assert tempered.kd.item() == pytest.approx(0.32255033260285915, abs=1e-5)
    assert tempered.ce.item() == pytest.approx(1.2039728043259361, abs=1e-6)


def test_tail_beta_amplifies_only_the_tail_term():
    """Doubling beta adds one more copy of the tail term: 0.3111987 - 2*0.1732868."""
    amplified = distillation_loss(
        **_batch(), config=DistillLossConfig(kd_alpha=1.0, ce_alpha=0.0, tail_beta=2.0)
    )

    assert amplified.kd.item() == pytest.approx(
        0.3111986997115478 - 2 * 0.17328679513998632, abs=1e-6
    )


def test_kd_is_averaged_over_supervised_tokens():
    """Two identical supervised positions average to one position's loss."""
    batch = _batch(prompt_len=1, seq_len=3)
    batch["student_logits"][0, 1] = batch["student_logits"][0, 0]
    for name in ("labels", "teacher_support_len", "teacher_tail_mass"):
        batch[name][0, 2] = batch[name][0, 1]
    for name in ("teacher_token_ids", "teacher_logprobs"):
        batch[name][0, 2] = batch[name][0, 1]

    parts = distillation_loss(**batch, config=KD_ONLY)

    assert parts.supervised_tokens == 2
    assert parts.kd.item() == pytest.approx(0.13791190457156147, abs=1e-6)


def test_gradient_reaches_the_student_and_stays_finite():
    batch = _batch()
    batch["student_logits"].requires_grad_(True)

    distillation_loss(**batch).loss.backward()

    grad = batch["student_logits"].grad
    assert torch.isfinite(grad).all()
    assert grad[0, 0].abs().sum().item() > 0.0
    assert grad[0, 1].abs().sum().item() == 0.0


def test_supervision_disagreement_is_refused():
    batch = _batch()
    batch["teacher_support_len"][0, 1] = 0

    with pytest.raises(ValueError, match="misaligned"):
        distillation_loss(**batch)


def test_supervising_position_zero_is_refused():
    batch = _batch(seq_len=2)
    batch["labels"][0, 0] = 3

    with pytest.raises(ValueError, match="position 0"):
        distillation_loss(**batch)


def test_shape_mismatch_is_refused():
    batch = _batch()
    batch["teacher_tail_mass"] = batch["teacher_tail_mass"][:, :1]

    with pytest.raises(ValueError, match="teacher_tail_mass"):
        distillation_loss(**batch)


def test_chunking_does_not_change_the_result():
    batch = _batch(prompt_len=1, seq_len=3)
    batch["student_logits"][0, 1] = batch["student_logits"][0, 0]
    for name in ("labels", "teacher_support_len", "teacher_tail_mass"):
        batch[name][0, 2] = batch[name][0, 1]
    for name in ("teacher_token_ids", "teacher_logprobs"):
        batch[name][0, 2] = batch[name][0, 1]

    whole = distillation_loss(**batch, config=DistillLossConfig(chunk_rows=8))
    chunked = distillation_loss(**batch, config=DistillLossConfig(chunk_rows=1))

    assert chunked.loss.item() == pytest.approx(whole.loss.item(), abs=1e-6)


@pytest.mark.parametrize(
    "kwargs",
    [
        {"temperature": 0.0},
        {"kd_alpha": -0.1},
        {"kd_alpha": 0.0, "ce_alpha": 0.0},
        {"tail_beta": -1.0},
        {"chunk_rows": 0},
    ],
)
def test_invalid_config_is_refused(kwargs):
    with pytest.raises(ValueError):
        DistillLossConfig(**kwargs)


def test_config_from_hyperparams_uses_documented_defaults():
    default = DistillLossConfig.from_hyperparams({})

    assert (default.kd_alpha, default.ce_alpha) == (0.9, 0.1)
    assert (default.temperature, default.tail_beta) == (1.0, 1.0)

    overridden = DistillLossConfig.from_hyperparams({"kd_alpha": 0.5, "tail_beta": 2.0})
    assert (overridden.kd_alpha, overridden.tail_beta) == (0.5, 2.0)


def _record(*, tokens, completion_start, support, tail):
    np = pytest.importorskip("numpy")
    width = len(support[0])
    return {
        "input_ids": np.asarray(tokens, dtype=np.uint32),
        "completion_start": completion_start,
        "token_ids": np.asarray([[t for t, _ in row] for row in support], dtype=np.uint32),
        "logprobs": np.asarray(
            [[math.log(p) for _, p in row] for row in support], dtype=np.float16
        ),
        "support_len": np.full(len(support), width, dtype=np.uint16),
        "tail_mass": np.asarray(tail, dtype=np.float16),
    }


def test_collate_pads_and_masks_a_ragged_batch():
    short = _record(
        tokens=[5, 6, 7], completion_start=2, support=[[(7, 0.5), (8, 0.25)]], tail=[0.25]
    )
    long = _record(
        tokens=[5, 6, 7, 8],
        completion_start=2,
        support=[[(7, 0.5), (8, 0.25)], [(8, 0.5), (9, 0.25)]],
        tail=[0.25, 0.25],
    )

    batch = collate_distill_batch([short, long], pad_token_id=99)

    assert batch["input_ids"][0].tolist() == [5, 6, 7, 99]
    assert batch["attention_mask"][0].tolist() == [1, 1, 1, 0]
    assert batch["labels"][0].tolist() == [IGNORE_INDEX, IGNORE_INDEX, 7, IGNORE_INDEX]
    assert batch["labels"][1].tolist() == [IGNORE_INDEX, IGNORE_INDEX, 7, 8]
    assert batch["teacher_support_len"][0].tolist() == [0, 0, 2, 0]

    logits = torch.zeros((2, 4, 16), dtype=torch.float32)
    parts = distillation_loss(
        student_logits=logits,
        labels=batch["labels"],
        teacher_token_ids=batch["teacher_token_ids"],
        teacher_logprobs=batch["teacher_logprobs"],
        teacher_support_len=batch["teacher_support_len"],
        teacher_tail_mass=batch["teacher_tail_mass"],
    )
    assert parts.supervised_tokens == 3
    assert math.isfinite(parts.loss.item())


def test_collate_refuses_a_record_supervised_from_position_zero():
    record = _record(
        tokens=[5, 6], completion_start=0, support=[[(5, 0.9)], [(6, 0.9)]], tail=[0.1, 0.1]
    )

    with pytest.raises(ValueError, match="position 0"):
        collate_distill_batch([record], pad_token_id=0)
