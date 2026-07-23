"""Tests for on-policy, judge-filtered DPO preference-pair construction.

The rejected sample must be a genuine on-policy model generation (not the gold
answer truncated), degenerate pairs must be dropped, and a sample judged better
than gold must not be labelled 'rejected'.
"""

import sys
import types

import pytest


@pytest.fixture
def tm(monkeypatch):
    """Import train_model with a fake `datasets` module (ML-only dep, absent in dev)."""
    fake_datasets = types.ModuleType("datasets")

    class _DS:
        def __init__(self, d):
            self.d = d

        @staticmethod
        def from_dict(d):
            return _DS(d)

    fake_datasets.Dataset = _DS
    monkeypatch.setitem(sys.modules, "datasets", fake_datasets)
    import src.activities.train_model as tm

    return tm


class _LlmConfig:
    api_base_url = "http://x"
    api_key = "k"
    model = "m"


class _Tok:
    """Minimal tokenizer exposing apply_chat_template, as render_chat expects."""

    chat_template = "present"

    def apply_chat_template(self, messages, tokenize=False, add_generation_prompt=False):
        parts = [f"{m['role']}: {m['content']}" for m in messages]
        if add_generation_prompt:
            parts.append("assistant:")
        return "\n".join(parts)


def _install_fakes(monkeypatch, tm, *, generate, compare_ab):
    """Fake the model-inference backend and the judge used inside _create_dpo_pairs."""
    fake_inf = types.SimpleNamespace(
        generate=lambda model, tok, prompt, max_new_tokens=256: generate(prompt)
    )
    monkeypatch.setattr("src.backends.model_inference.get", lambda name="hf": fake_inf)

    class _Judge:
        def __init__(self, *a, **k):
            pass

        def compare_ab(self, prompt, a, b):
            return compare_ab(a, b)

    monkeypatch.setattr(tm, "OpenAICompatibleJudge", _Judge)


def _dataset(examples):
    return {"messages": examples}


def _ex(prompt, response):
    return [
        {"role": "user", "content": prompt},
        {"role": "assistant", "content": response},
    ]


def test_rejected_is_on_policy_generation_not_truncation(monkeypatch, tm):
    # Model generates a clearly different (worse) answer; judge prefers gold ("A").
    _install_fakes(
        monkeypatch,
        tm,
        generate=lambda prompt: "wrong nonsense answer",
        compare_ab=lambda gold, sample: "A",
    )
    ds = _dataset([_ex("2+2?", "The answer is 4.")])
    out = tm._create_dpo_pairs(object(), ds, _Tok(), {}, _LlmConfig(), settings=None)

    chosen = out.d["chosen"][0]
    rejected = out.d["rejected"][0]
    assert "The answer is 4." in chosen
    assert "wrong nonsense answer" in rejected
    # rejected must NOT be a truncation of the gold answer
    assert "wrong nonsense answer" not in "The answer is 4."


def test_degenerate_identical_pair_dropped(monkeypatch, tm):
    # Model reproduces the gold answer verbatim → no preference signal → dropped.
    _install_fakes(
        monkeypatch,
        tm,
        generate=lambda prompt: "The answer is 4.",
        compare_ab=lambda gold, sample: "A",
    )
    ds = _dataset([_ex("2+2?", "The answer is 4.")])
    with pytest.raises(ValueError, match="no usable preference pairs"):
        tm._create_dpo_pairs(object(), ds, _Tok(), {}, _LlmConfig(), settings=None)


def test_sample_judged_better_is_dropped(monkeypatch, tm):
    # Judge says the sampled response ("B") beats gold → must not label it rejected.
    _install_fakes(
        monkeypatch,
        tm,
        generate=lambda prompt: "a genuinely better answer",
        compare_ab=lambda gold, sample: "B",
    )
    ds = _dataset([_ex("q", "mediocre gold")])
    with pytest.raises(ValueError, match="no usable preference pairs"):
        tm._create_dpo_pairs(object(), ds, _Tok(), {}, _LlmConfig(), settings=None)


def test_max_dpo_pairs_caps_generation(monkeypatch, tm):
    gen_calls = {"n": 0}

    def gen(prompt):
        gen_calls["n"] += 1
        return f"sample {gen_calls['n']}"

    _install_fakes(monkeypatch, tm, generate=gen, compare_ab=lambda gold, sample: "A")
    ds = _dataset([_ex(f"q{i}", f"gold {i}") for i in range(5)])
    out = tm._create_dpo_pairs(
        object(), ds, _Tok(), {"max_dpo_pairs": 2}, _LlmConfig(), settings=None
    )
    assert len(out.d["chosen"]) == 2


def test_judge_filter_disabled_keeps_all(monkeypatch, tm):
    calls = {"judge": 0}

    def cmp(gold, sample):
        calls["judge"] += 1
        return "B"  # would drop everything if consulted

    _install_fakes(monkeypatch, tm, generate=lambda p: "diff answer", compare_ab=cmp)
    ds = _dataset([_ex("q", "gold answer")])
    out = tm._create_dpo_pairs(
        object(), ds, _Tok(), {"dpo_judge_filter": False}, _LlmConfig(), settings=None
    )
    assert len(out.d["chosen"]) == 1
    assert calls["judge"] == 0  # judge not consulted when filter disabled
