"""Tests for the ModalGpuProvider eval/iterative methods + LocalGpuProvider delegation.

Focus on the behaviors unique to this PR:
  - iterative methods (run_sft_round, run_evaluate_holdout) CLEAR the reservation
    after completion, so the next sequential round does not recover a finished call
  - run_evaluation reserves against the `evaluations` table (not training_jobs) and
    does NOT clear (single call per evaluation)
  - recovery: an existing modal_call_id short-circuits to FunctionCall.from_id
  - LocalGpuProvider methods delegate to the matching pure-compute core
"""

import sys
import types

import pytest


def _install_fake_modal(monkeypatch, *, spawn_result, from_id_result=None):
    calls = {"spawn": 0, "from_id": 0, "gpu": None, "fn_name": None}

    class _FunctionCallHandle:
        def __init__(self, source):
            self._source = source
            self._polls = 0
            self.object_id = "call-xyz"
            self.get = types.SimpleNamespace(aio=self._get)

        async def _get(self, timeout=0):
            if self._source == "from_id":
                return from_id_result
            self._polls += 1
            if self._polls <= 1:
                raise TimeoutError
            return spawn_result

    class _FunctionHandle:
        def with_options(self, gpu=None):
            calls["gpu"] = gpu
            return self

        async def _spawn(self, payload):
            calls["spawn"] += 1
            return _FunctionCallHandle("spawn")

        def __init__(self):
            self.spawn = types.SimpleNamespace(aio=self._spawn)

    _fn_handle = _FunctionHandle()

    class FunctionCall:
        @staticmethod
        def from_id(call_id):
            calls["from_id"] += 1
            return _FunctionCallHandle("from_id")

    class Function:
        @staticmethod
        def from_name(app, fn):
            calls["fn_name"] = fn
            return _fn_handle

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


class _Settings:
    modal_app_name = "app"
    modal_function_name = "train"
    modal_sft_round_function_name = "train_sft_round"
    modal_evaluate_holdout_function_name = "evaluate_holdout"
    modal_evaluation_function_name = "run_evaluation"
    modal_poll_interval_secs = 0


def _make_provider(db):
    import src.gpu_provider as gp

    class _Infra:
        settings = _Settings()

    infra = _Infra()
    infra.db = db
    return gp.ModalGpuProvider(infra)


async def _noop_sleep(*_a, **_k):
    return None


def _patch_common(monkeypatch):
    monkeypatch.setattr("asyncio.sleep", _noop_sleep)
    monkeypatch.setattr("temporalio.activity.heartbeat", lambda *a, **k: None)


@pytest.mark.asyncio
async def test_sft_round_clears_reservation_after_completion(monkeypatch):
    _patch_common(monkeypatch)
    calls = _install_fake_modal(
        monkeypatch,
        spawn_result={"adapter_path": "s3://ckpt", "adapter_size_bytes": 9, "metrics": {}},
    )
    db = _FakeDB(existing_call_id=None)
    prov = _make_provider(db)

    out = await prov.run_sft_round(
        tenant_id="t",
        training_job_id="j",
        dataset_path="p",
        base_model="m",
        method="lora",
        hyperparams={},
        iteration=1,
        adapter_path="s3://prev",
        gpu_class="A10G",
    )

    assert out["adapter_path"] == "s3://ckpt"
    assert calls["spawn"] == 1
    assert calls["fn_name"] == "train_sft_round"
    # Two writes: persist-before-poll, then clear-after-completion.
    assert len(db.updates) == 2, db.updates
    persist_q, persist_args = db.updates[0]
    clear_q, clear_args = db.updates[1]
    assert "training_jobs" in persist_q and persist_args[0] == "train_sft_round:call-xyz"
    assert "modal_call_id = NULL" in clear_q
    assert "training_jobs" in clear_q


@pytest.mark.asyncio
async def test_holdout_clears_reservation_after_completion(monkeypatch):
    _patch_common(monkeypatch)
    calls = _install_fake_modal(
        monkeypatch, spawn_result={"eval_loss": 0.4, "metrics": {"eval_loss": 0.4}}
    )
    db = _FakeDB(existing_call_id=None)
    prov = _make_provider(db)

    out = await prov.run_evaluate_holdout(
        tenant_id="t",
        training_job_id="j",
        adapter_path="s3://ckpt",
        base_model="m",
        method="lora",
        dataset_path="p",
        hyperparams={},
        iteration=1,
        gpu_class="A10G",
    )

    assert out["eval_loss"] == 0.4
    assert calls["fn_name"] == "evaluate_holdout"
    assert len(db.updates) == 2
    assert "modal_call_id = NULL" in db.updates[1][0]


@pytest.mark.asyncio
async def test_evaluation_reserves_on_evaluations_table_and_does_not_clear(monkeypatch):
    _patch_common(monkeypatch)
    calls = _install_fake_modal(monkeypatch, spawn_result={"scores": {"overall": 80}, "report": {}})
    db = _FakeDB(existing_call_id=None)
    prov = _make_provider(db)

    out = await prov.run_evaluation(
        tenant_id="t",
        model_id="model-1",
        evaluation_id="eval-1",
        adapter_path="s3://adapter",
        base_model="m",
        dataset_path="p",
        judge_model="jm",
        judge_api_base="jb",
        gpu_class="a10080gb",
        llm_config={
            "api_base_url": "u",
            "api_key": "k",
            "model": "m",
            "max_tokens": 1,
            "is_custom": False,
        },
    )

    assert out["scores"]["overall"] == 80
    assert calls["fn_name"] == "run_evaluation"
    assert calls["gpu"] == "A100-80GB"
    # Exactly one write (reservation); evaluation is single-shot → no clear.
    assert len(db.updates) == 1, db.updates
    persist_q, persist_args = db.updates[0]
    assert "evaluations" in persist_q
    assert persist_args[0] == "run_evaluation:call-xyz"
    assert persist_args[1] == "eval-1"


@pytest.mark.asyncio
async def test_sft_round_recovers_without_respawn(monkeypatch):
    _patch_common(monkeypatch)
    calls = _install_fake_modal(
        monkeypatch,
        spawn_result={"x": 1},
        from_id_result={"adapter_path": "s3://recovered", "adapter_size_bytes": 2, "metrics": {}},
    )
    # A same-function tagged reservation is recovered (retry / worker-restart).
    db = _FakeDB(existing_call_id="train_sft_round:call-existing")
    prov = _make_provider(db)

    out = await prov.run_sft_round(
        tenant_id="t",
        training_job_id="j",
        dataset_path="p",
        base_model="m",
        method="lora",
        hyperparams={},
        iteration=0,
        adapter_path=None,
        gpu_class="A10G",
    )

    assert out["adapter_path"] == "s3://recovered"
    assert calls["spawn"] == 0
    assert calls["from_id"] == 1
    # Recovery must not re-persist a call id; the only write is the post-run clear
    # (iterative rounds clear so the next round does not recover this finished call).
    assert len(db.updates) == 1
    assert "modal_call_id = NULL" in db.updates[0][0]


@pytest.mark.asyncio
async def test_sft_round_ignores_stale_holdout_reservation(monkeypatch):
    """The Critical regression guard: a stale holdout reservation left on the
    shared training_jobs.modal_call_id (e.g. a tolerated holdout failure that
    never cleared) must NOT be recovered by the next round's train_sft_round —
    it must spawn a fresh training call instead."""
    _patch_common(monkeypatch)
    calls = _install_fake_modal(
        monkeypatch,
        spawn_result={"adapter_path": "s3://fresh", "adapter_size_bytes": 5, "metrics": {}},
        from_id_result={"eval_loss": 0.9, "metrics": {}},  # would be WRONG to return here
    )
    db = _FakeDB(existing_call_id="evaluate_holdout:stale-call")
    prov = _make_provider(db)

    out = await prov.run_sft_round(
        tenant_id="t",
        training_job_id="j",
        dataset_path="p",
        base_model="m",
        method="lora",
        hyperparams={},
        iteration=1,
        adapter_path="s3://prev",
        gpu_class="A10G",
    )

    # Must have spawned fresh (not recovered the holdout call) and returned training output.
    assert out["adapter_path"] == "s3://fresh"
    assert calls["spawn"] == 1
    assert calls["from_id"] == 0
    # First write overwrites the stale reservation with this function's tagged id.
    assert db.updates[0][1][0] == "train_sft_round:call-xyz"


def test_recoverable_call_id_is_type_safe():
    import src.gpu_provider as gp

    prov = gp.ModalGpuProvider.__new__(gp.ModalGpuProvider)  # bypass __init__

    class _Infra:
        settings = _Settings()

    prov.infra = _Infra()

    # nothing stored → spawn fresh
    assert prov._recoverable_call_id(None, "train_sft_round") is None
    # same-function tag → recover the bare id
    assert prov._recoverable_call_id("train_sft_round:abc", "train_sft_round") == "abc"
    # different-function tag → do NOT recover (spawn fresh)
    assert prov._recoverable_call_id("evaluate_holdout:abc", "train_sft_round") is None
    # legacy bare id → only the single-shot training function owns it
    assert prov._recoverable_call_id("legacy-bare", "train") == "legacy-bare"
    assert prov._recoverable_call_id("legacy-bare", "train_sft_round") is None


# -- LocalGpuProvider delegation --


class _FakeCoreResult:
    def __init__(self, **kw):
        self.__dict__.update(kw)


def _make_local_provider():
    import src.gpu_provider as gp

    class _Infra:
        s3 = object()
        s3_bucket = "bucket"
        settings = object()

    return gp.LocalGpuProvider(_Infra())


@pytest.mark.asyncio
async def test_local_run_sft_round_delegates_to_core(monkeypatch):
    import src.activities.train_model as tm

    captured = {}

    async def _fake_core(inp, *, s3, s3_bucket, settings):
        captured["inp"] = inp
        captured["s3_bucket"] = s3_bucket
        return _FakeCoreResult(adapter_path="s3://x", adapter_size_bytes=7, metrics={"a": 1})

    monkeypatch.setattr(tm, "run_sft_round_core", _fake_core)
    prov = _make_local_provider()

    out = await prov.run_sft_round(
        tenant_id="t",
        training_job_id="j",
        dataset_path="p",
        base_model="m",
        method="lora",
        hyperparams={"r": 8},
        iteration=2,
        adapter_path="s3://prev",
        gpu_class="A10G",
    )

    assert out == {"adapter_path": "s3://x", "adapter_size_bytes": 7, "metrics": {"a": 1}}
    assert captured["s3_bucket"] == "bucket"
    assert captured["inp"].iteration == 2
    assert captured["inp"].adapter_path == "s3://prev"


@pytest.mark.asyncio
async def test_local_run_evaluation_delegates_with_reconstructed_llm_config(monkeypatch):
    import src.activities.run_evaluation as re_mod

    captured = {}

    async def _fake_core(inp, *, s3, s3_bucket, settings, llm_config):
        captured["inp"] = inp
        captured["llm_config"] = llm_config
        return _FakeCoreResult(scores={"overall": 42}, report={"ok": True})

    monkeypatch.setattr(re_mod, "run_evaluation_core", _fake_core)
    prov = _make_local_provider()

    out = await prov.run_evaluation(
        tenant_id="t",
        model_id="model-1",
        evaluation_id="eval-1",
        adapter_path="s3://adapter",
        base_model="m",
        dataset_path="p",
        judge_model="jm",
        judge_api_base="jb",
        gpu_class="A10G",
        llm_config={
            "api_base_url": "u",
            "api_key": "k",
            "model": "jm",
            "max_tokens": 100,
            "is_custom": True,
        },
    )

    assert out == {"scores": {"overall": 42}, "report": {"ok": True}}
    # dict was reconstructed into a TenantLlmConfig dataclass
    assert captured["llm_config"].api_base_url == "u"
    assert captured["llm_config"].is_custom is True
    assert captured["inp"].evaluation_id == "eval-1"
