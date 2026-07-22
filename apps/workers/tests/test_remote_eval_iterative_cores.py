"""Tests for the pure-compute cores extracted for the remote eval/iterative path.

Each core (run_sft_round_core, run_evaluate_holdout_core, run_evaluation_core)
must be DB-free / Redis-free: it takes only s3 + settings (+ llm_config where a
judge LLM is used) and never an infra/db handle. That boundary is what lets the
same function run in-process (LocalGpuProvider) and inside a remote Modal
container. These tests enforce the boundary and drive the fully-stubbable core.
"""

import inspect

import pytest

from src.activities.stubs import (
    EvaluateHoldoutInput,
    RunEvaluationInput,
    TrainSftRoundInput,
)


def test_sft_round_core_signature_is_pure_compute():
    from src.activities.train_model import run_sft_round_core

    params = inspect.signature(run_sft_round_core).parameters
    assert {"s3", "s3_bucket", "settings"} <= set(params)
    assert "infra" not in params
    assert "db" not in params


def test_holdout_core_signature_is_pure_compute():
    from src.activities.train_model import run_evaluate_holdout_core

    params = inspect.signature(run_evaluate_holdout_core).parameters
    assert {"s3", "s3_bucket", "settings"} <= set(params)
    assert "infra" not in params
    assert "db" not in params


def test_evaluation_core_signature_takes_llm_config_and_no_infra():
    from src.activities.run_evaluation import run_evaluation_core

    params = inspect.signature(run_evaluation_core).parameters
    assert {"s3", "s3_bucket", "settings", "llm_config"} <= set(params)
    # The DB-backed tenant-config lookup moved to the worker activity.
    assert "infra" not in params
    assert "db" not in params


def test_run_evaluation_input_gpu_class_defaults_to_none():
    inp = RunEvaluationInput(
        tenant_id="t",
        model_id="m",
        evaluation_id="e",
        adapter_path="a",
        base_model="b",
        dataset_path="d",
    )
    assert inp.gpu_class is None


def test_evaluate_holdout_input_gpu_class_defaults_to_none():
    inp = EvaluateHoldoutInput(
        tenant_id="t",
        training_job_id="j",
        adapter_path="a",
        base_model="b",
        method="lora",
        dataset_path="d",
        hyperparams={},
        iteration=0,
    )
    assert inp.gpu_class is None


class _FakeEngine:
    def load_model(self, **k):
        return ("model", "tok")

    def attach_adapter(self, model, **k):
        return model

    def save_adapter(self, model, tok, path):
        path.mkdir(parents=True, exist_ok=True)
        (path / "adapter.txt").write_text("x")


@pytest.mark.asyncio
async def test_sft_round_core_runs_without_db(monkeypatch):
    """Drive run_sft_round_core with every heavy piece stubbed; assert it needs no DB."""
    import src.activities.train_model as tm

    monkeypatch.setattr(tm, "get_engine", lambda s: _FakeEngine())
    monkeypatch.setattr(tm, "_download_dataset", lambda *a, **k: None)
    monkeypatch.setattr(tm, "_load_chatml_dataset", lambda p: [{"messages": []}])
    monkeypatch.setattr(tm, "_get_metrics_collector", lambda s: None)
    monkeypatch.setattr(
        tm, "_train_sft", lambda *a, **k: {"iter_0_train_loss": 1.23, "iter_0_train_runtime": 2.0}
    )
    monkeypatch.setattr(tm, "_upload_adapter", lambda *a, **k: 4242)

    inp = TrainSftRoundInput(
        tenant_id="tenant-1",
        training_job_id="job-1",
        dataset_path="datasets/x.jsonl",
        base_model="m",
        method="lora",
        hyperparams={},
        iteration=0,
        adapter_path=None,
        gpu_class="A10G",
    )

    out = await tm.run_sft_round_core(inp, s3=object(), s3_bucket="b", settings=object())

    assert out.adapter_size_bytes == 4242
    assert out.adapter_path.endswith("iter-0/")
    assert out.metrics["iter_0_train_loss"] == 1.23


@pytest.mark.asyncio
async def test_holdout_core_runs_without_db(monkeypatch):
    """Drive run_evaluate_holdout_core with heavy pieces stubbed; assert no DB needed."""
    import src.activities.train_model as tm

    class _ModelWithAdapter:
        def load_adapter(self, *a, **k):
            return None

    class _Engine(_FakeEngine):
        def load_model(self, **k):
            return (_ModelWithAdapter(), "tok")

        def attach_adapter(self, model, **k):
            return model

    monkeypatch.setattr(tm, "get_engine", lambda s: _Engine())
    monkeypatch.setattr(tm, "_download_dataset", lambda *a, **k: None)
    monkeypatch.setattr(tm, "_load_chatml_dataset", lambda p: [{"messages": []}])
    monkeypatch.setattr(tm, "_download_adapter", lambda *a, **k: None)
    monkeypatch.setattr(tm, "_get_metrics_collector", lambda s: None)
    monkeypatch.setattr(tm, "_stream_metric", lambda *a, **k: None)
    monkeypatch.setattr(tm, "_evaluate_on_holdout", lambda *a, **k: 0.5)

    inp = EvaluateHoldoutInput(
        tenant_id="tenant-1",
        training_job_id="job-1",
        adapter_path="checkpoints/tenant-1/job-1/iter-0/",
        base_model="m",
        method="lora",
        dataset_path="datasets/x.jsonl",
        hyperparams={},
        iteration=0,
        gpu_class="A10G",
    )

    out = await tm.run_evaluate_holdout_core(inp, s3=object(), s3_bucket="b", settings=object())

    assert out.eval_loss == 0.5
    assert out.metrics["eval_loss"] == 0.5
    assert out.metrics["iteration"] == 0
