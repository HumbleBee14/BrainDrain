"""Modes that score with the judge must be preflighted before GPU dispatch."""

import ast
from pathlib import Path

from src.activities.train_model import _JUDGE_BACKED_MODES

_TRAIN_MODEL = Path(__file__).resolve().parents[1] / "src" / "activities" / "train_model.py"
_STRATEGIES = Path(__file__).resolve().parents[1] / "src" / "activities" / "train_model.py"


def _registered_strategies() -> set[str]:
    tree = ast.parse(_STRATEGIES.read_text())
    names = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.ClassDef):
            continue
        for dec in node.decorator_list:
            if (
                isinstance(dec, ast.Call)
                and getattr(dec.func, "id", None) == "register_strategy"
                and dec.args
                and isinstance(dec.args[0], ast.Constant)
            ):
                names.add(dec.args[0].value)
    return names


def test_judge_backed_modes_are_real_strategies():
    assert _JUDGE_BACKED_MODES <= _registered_strategies()


def test_dpo_and_grpo_strategies_are_judge_backed():
    source = _STRATEGIES.read_text()
    tree = ast.parse(source)
    for node in ast.walk(tree):
        if not isinstance(node, ast.ClassDef):
            continue
        modes = [
            d.args[0].value
            for d in node.decorator_list
            if isinstance(d, ast.Call)
            and getattr(d.func, "id", None) == "register_strategy"
            and d.args
            and isinstance(d.args[0], ast.Constant)
        ]
        if not modes:
            continue
        body = ast.get_source_segment(source, node) or ""
        # A strategy that runs DPO or GRPO scores with the judge, so it must be
        # listed for preflight or a missing key surfaces only after GPU spend.
        if "_train_dpo" in body or "_train_grpo" in body:
            for mode in modes:
                assert mode in _JUDGE_BACKED_MODES, f"{mode} uses the judge but is not preflighted"


def test_preflight_runs_before_gpu_dispatch():
    source = _TRAIN_MODEL.read_text()
    preflight = source.index("_JUDGE_BACKED_MODES:")
    dispatch = source.index("await self.gpu_provider.run_training(")
    assert preflight < dispatch
