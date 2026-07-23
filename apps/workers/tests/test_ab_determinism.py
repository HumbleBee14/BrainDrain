"""The A/B comparison suite must be reproducible.

Response positions are still randomized per sample to cancel position bias, but
seeded from a fixed value so two evaluations of the same model on the same data
produce the same win rate instead of drifting with the global RNG.
"""


class _Tok:
    def apply_chat_template(self, messages, tokenize=False, add_generation_prompt=False):
        return "PROMPT"


class _Judge:
    """Deterministic judge: prefers whichever side is the fine-tuned response."""

    def compare_ab(self, prompt, resp_a, resp_b):
        return "A" if "ft" in resp_a else "B"


def _val_dataset(n=8):
    return [
        {
            "messages": [
                {"role": "user", "content": f"q{i}"},
                {"role": "assistant", "content": "gold"},
            ]
        }
        for i in range(n)
    ]


def test_ab_comparison_is_deterministic(monkeypatch):
    import src.activities.run_evaluation as re

    # model_ft == "ft" -> "ft-response"; model_base == "base" -> "base-response".
    monkeypatch.setattr(
        re,
        "_generate",
        lambda model, tok, prompt, **k: f"{model}-response",
    )

    suite = re.ABComparisonSuite()
    scores_1, report_1 = suite.run("ft", _Tok(), "base", _Tok(), _Judge(), _val_dataset())
    scores_2, report_2 = suite.run("ft", _Tok(), "base", _Tok(), _Judge(), _val_dataset())

    assert scores_1["win_rate"] == scores_2["win_rate"]
    assert scores_1 == scores_2
    assert report_1["comparisons"] == report_2["comparisons"]
    # Sanity: the fine-tuned model wins every decisive comparison here.
    assert scores_1["win_rate"] == 1.0
