import inspect

import pytest

from src.gpu_provider import GpuProvider, LocalGpuProvider


def test_protocol_run_training_has_llm_config():
    sig = inspect.signature(GpuProvider.run_training)
    assert "llm_config" in sig.parameters


@pytest.mark.asyncio
async def test_local_provider_forwards_to_core(monkeypatch):
    captured = {}

    async def _fake_core(inp, *, s3, s3_bucket, settings, llm_config):
        captured["llm_config"] = llm_config
        captured["s3_bucket"] = s3_bucket

        class _O:
            adapter_path = "s3://a"
            adapter_size_bytes = 7
            metrics = {"m": 1}

        return _O()

    # Patch where LocalGpuProvider imports it (function-local import in run_training).
    monkeypatch.setattr("src.activities.train_model.run_training_core", _fake_core)

    class _Infra:
        s3 = object()
        s3_bucket = "bkt"

        class settings:
            pass

    prov = LocalGpuProvider(_Infra())
    out = await prov.run_training(
        tenant_id="t",
        training_job_id="j",
        dataset_path="p",
        base_model="m",
        method="lora",
        mode="quick",
        hyperparams={},
        gpu_class="A10G",
        llm_config={
            "api_base_url": "http://x",
            "api_key": "k",
            "model": "m",
            "max_tokens": 10,
            "is_custom": False,
        },
    )
    assert out["adapter_size_bytes"] == 7
    # llm_config dict is reconstructed into TenantLlmConfig for the core.
    from src.tenant_config import TenantLlmConfig

    assert isinstance(captured["llm_config"], TenantLlmConfig)
    assert captured["s3_bucket"] == "bkt"
