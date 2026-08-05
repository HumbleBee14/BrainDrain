"""Tests for ModalGpuProvider: spawn/poll + durable reservation.

Exercises the three required behaviors without a real `modal` install:
  1. Spawn-once + persist-before-poll (no existing call_id).
  2. Recover-no-respawn (existing modal_call_id short-circuits to FunctionCall.from_id).
  3. GPU class mapped through MODAL_GPU_MAP and forwarded to .with_options(gpu=...).
"""

import sys
import types

import pytest


def _install_fake_modal(
    monkeypatch, *, spawn_result, timeouts_before_result=1, from_id_result=None
):
    """Install a fake `modal` module matching ModalGpuProvider's real call sites:

    - Function.from_name(app, fn).with_options(gpu=...).spawn.aio(payload) -> FunctionCall
    - modal.FunctionCall.from_id(call_id) -> FunctionCall-like
    - FunctionCall-like exposes .object_id and .get.aio(timeout=0)
    """
    calls = {"spawn": 0, "from_id": 0, "gpu": None, "from_name": None}

    class _FunctionCallHandle:
        """What .spawn.aio(...) or FunctionCall.from_id(...) returns."""

        def __init__(self, source):
            self._source = source
            self._polls = 0
            self.object_id = "call-xyz"
            self.get = types.SimpleNamespace(aio=self._get)

        async def _get(self, timeout=0):
            if self._source == "from_id":
                return from_id_result
            self._polls += 1
            if self._polls <= timeouts_before_result:
                raise TimeoutError
            return spawn_result

    class _FunctionHandle:
        """What Function.from_name(...) returns."""

        def with_options(self, gpu=None):
            calls["gpu"] = gpu
            return self

        async def _spawn(self, payload):
            calls["spawn"] += 1
            return _FunctionCallHandle("spawn")

        def __init__(self):
            self.spawn = types.SimpleNamespace(aio=self._spawn)

    _function_handle = _FunctionHandle()

    class FunctionCall:
        @staticmethod
        def from_id(call_id):
            calls["from_id"] += 1
            return _FunctionCallHandle("from_id")

    class Function:
        @staticmethod
        def from_name(app, fn):
            calls["from_name"] = (app, fn)
            return _function_handle

    fake = types.ModuleType("modal")
    fake.Function = Function
    fake.FunctionCall = FunctionCall
    monkeypatch.setitem(sys.modules, "modal", fake)
    return calls


class _FakeDB:
    def __init__(self, existing_call_id=None):
        self.existing = existing_call_id
        self.updates = []

    async def fetchval(self, q, *args):
        return self.existing

    async def execute(self, q, *args):
        self.updates.append((q, args))


def _make_provider(db, function_apps=None):
    import src.gpu_provider as gp

    class _S:
        modal_app_name = "app"
        modal_function_apps = function_apps or {}
        modal_function_name = "train"
        modal_poll_interval_secs = 0

    class _Infra:
        settings = _S()

    infra = _Infra()
    infra.db = db
    return gp.ModalGpuProvider(infra)


async def _noop_sleep(*_a, **_k):
    return None


@pytest.mark.asyncio
async def test_spawns_and_persists_call_id_before_poll(monkeypatch):
    monkeypatch.setattr("asyncio.sleep", _noop_sleep)
    calls = _install_fake_modal(
        monkeypatch,
        spawn_result={"adapter_path": "s3://a", "adapter_size_bytes": 1, "metrics": {}},
    )
    monkeypatch.setattr("temporalio.activity.heartbeat", lambda *a, **k: None)
    db = _FakeDB(existing_call_id=None)
    prov = _make_provider(db)

    out = await prov.run_training(
        tenant_id="t",
        training_job_id="j",
        dataset_path="p",
        base_model="m",
        method="lora",
        mode="quick",
        hyperparams={},
        gpu_class="a10080gb",
        llm_config={
            "api_base_url": "u",
            "api_key": "k",
            "model": "m",
            "max_tokens": 1,
            "is_custom": False,
        },
    )

    assert out["adapter_path"] == "s3://a"
    assert calls["spawn"] == 1
    assert calls["from_id"] == 0
    assert calls["gpu"] == "A100-80GB"
    assert db.updates, "call_id must be persisted before polling"
    # persisted value must be the spawned object_id, tagged with the function name
    _, args = db.updates[0]
    assert args[0] == "train:call-xyz"


@pytest.mark.asyncio
async def test_function_app_override_routes_to_other_app(monkeypatch):
    monkeypatch.setattr("asyncio.sleep", _noop_sleep)
    calls = _install_fake_modal(
        monkeypatch,
        spawn_result={"adapter_path": "s3://a", "adapter_size_bytes": 1, "metrics": {}},
    )
    monkeypatch.setattr("temporalio.activity.heartbeat", lambda *a, **k: None)
    prov = _make_provider(
        _FakeDB(existing_call_id=None), function_apps={"train": "side-app"}
    )

    await prov.run_training(
        tenant_id="t",
        training_job_id="j",
        dataset_path="p",
        base_model="m",
        method="lora",
        mode="quick",
        hyperparams={},
        gpu_class="a10080gb",
        llm_config={
            "api_base_url": "u",
            "api_key": "k",
            "model": "m",
            "max_tokens": 1,
            "is_custom": False,
        },
    )

    assert calls["from_name"] == ("side-app", "train")


@pytest.mark.asyncio
async def test_recovers_without_respawn(monkeypatch):
    monkeypatch.setattr("asyncio.sleep", _noop_sleep)
    calls = _install_fake_modal(
        monkeypatch,
        spawn_result={"x": 1},
        from_id_result={"adapter_path": "s3://recovered", "adapter_size_bytes": 2, "metrics": {}},
    )
    monkeypatch.setattr("temporalio.activity.heartbeat", lambda *a, **k: None)
    db = _FakeDB(existing_call_id="call-existing")
    prov = _make_provider(db)

    out = await prov.run_training(
        tenant_id="t",
        training_job_id="j",
        dataset_path="p",
        base_model="m",
        method="lora",
        mode="quick",
        hyperparams={},
        gpu_class="a10g",
        llm_config={
            "api_base_url": "u",
            "api_key": "k",
            "model": "m",
            "max_tokens": 1,
            "is_custom": False,
        },
    )

    assert out["adapter_path"] == "s3://recovered"
    assert calls["spawn"] == 0
    assert calls["from_id"] == 1
    assert not db.updates, "recovery path must not re-persist a call id"


@pytest.mark.asyncio
async def test_resolve_gpu_uses_modal_gpu_map():
    prov_cls = __import__("src.gpu_provider", fromlist=["ModalGpuProvider"]).ModalGpuProvider
    prov = prov_cls.__new__(prov_cls)  # bypass __init__ (no modal install needed here)
    assert prov._resolve_gpu("a10080gb") == "A100-80GB"
    assert prov._resolve_gpu("a10g") == "A10G"
    assert prov._resolve_gpu("h100") == "H100"
    # Canonical class is case-insensitive.
    assert prov._resolve_gpu("A10G") == "A10G"
    # Unknown/None fall back to the default-rate class.
    assert prov._resolve_gpu("unknown-class") == "T4"
    assert prov._resolve_gpu(None) == "T4"
