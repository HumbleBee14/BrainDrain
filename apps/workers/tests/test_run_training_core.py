import inspect

import pytest

from src.activities.stubs import StartTrainingInput
from src.tenant_config import TenantLlmConfig


def test_core_signature_takes_llm_config_and_no_infra():
    from src.activities.train_model import run_training_core

    sig = inspect.signature(run_training_core)
    params = sig.parameters
    assert "llm_config" in params
    assert "s3" in params
    assert "s3_bucket" in params
    assert "settings" in params
    # Must NOT take an infra/db handle — enforces the pure-compute boundary.
    assert "infra" not in params
    assert "db" not in params


@pytest.mark.asyncio
async def test_core_never_resolves_tenant_config_from_db(monkeypatch):
    """run_training_core must not call the DB-backed tenant config resolver."""
    import src.activities.train_model as tm

    called = {"hit": False}

    async def _boom(*a, **k):
        called["hit"] = True
        raise AssertionError("run_training_core must not read tenant config from DB")

    # If the core tried to resolve tenant config itself, this would fire.
    monkeypatch.setattr(tm, "get_tenant_llm_config", _boom, raising=False)

    # Stub the heavy pieces so we can drive the core without a GPU.
    class _FakeEngine:
        def load_model(self, **k):
            return ("model", "tok")

        def attach_adapter(self, model, **k):
            return model

        def save_adapter(self, model, tok, path):
            path.mkdir(parents=True, exist_ok=True)
            (path / "adapter.txt").write_text("x")

    class _FakeStrategy:
        def execute(self, **k):
            return {"train_runtime": 1.0}

    monkeypatch.setattr(tm, "get_engine", lambda s, **_: _FakeEngine())
    monkeypatch.setattr(tm, "get_strategy", lambda m: _FakeStrategy())
    monkeypatch.setattr(tm, "_download_dataset", lambda *a, **k: None)
    monkeypatch.setattr(tm, "_load_chatml_dataset", lambda p: [{"a": 1}])
    monkeypatch.setattr(tm, "_upload_adapter", lambda *a, **k: 123)
    monkeypatch.setattr(tm, "_get_metrics_collector", lambda s: None)

    class _S:
        s3_bucket = "b"

    inp = StartTrainingInput(
        tenant_id="t",
        training_job_id="j",
        dataset_path="p",
        base_model="m",
        method="lora",
        mode="quick",
        hyperparams={},
        gpu_class="A10G",
    )
    llm = TenantLlmConfig(
        api_base_url="http://x", api_key="k", model="m", max_tokens=10, is_custom=False
    )

    out = await tm.run_training_core(inp, s3=object(), s3_bucket="b", settings=_S(), llm_config=llm)
    assert out.adapter_size_bytes == 123
    assert called["hit"] is False
