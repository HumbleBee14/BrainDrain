"""Tests for BeamGpuProvider: submit/poll + durable reservation over Beam's REST API.

Mirrors test_modal_provider.py's required behaviors:
  1. Submit-once + persist-before-poll (no stored task id).
  2. Reattach-no-resubmit (stored tagged id short-circuits to a status GET).
  3. A stored id Beam doesn't know (404 — e.g. left by the Modal provider)
     falls back to a fresh submit instead of failing the job.
  4. Unmapped gpu_class raises instead of silently substituting hardware.
  5. A terminal non-COMPLETE status raises.
"""

import json
import types

import httpx
import pytest

import src.gpu_provider as gp


class _FakeDB:
    def __init__(self, existing=None):
        self.existing = existing
        self.updates = []

    async def fetchval(self, q, *args):
        return self.existing

    async def execute(self, q, *args):
        self.updates.append((q, args))


def _make_provider(db, queue_urls=None):
    class _S:
        beam_token = "tok"
        beam_workspace_id = "ws"
        beam_queue_urls = queue_urls or {"train": "https://train-queue.test"}
        beam_task_api_base = "https://api.test/task"
        beam_poll_interval_secs = 0

    infra = types.SimpleNamespace(settings=_S(), db=db)
    return gp.BeamGpuProvider(infra)


def _install_fake_http(monkeypatch, *, submit_task_id, statuses, results=None):
    """Route the provider's httpx calls to canned responses.

    `statuses` is consumed one GET at a time; `results` maps task id -> result
    payload returned alongside the final status. A None status entry produces
    a 404 (task unknown to Beam).
    """
    calls = {"posts": [], "gets": 0}
    status_iter = iter(statuses)

    class _Response:
        def __init__(self, code, body):
            self.status_code = code
            self._body = body

        def json(self):
            return self._body

        def raise_for_status(self):
            if self.status_code >= 400:
                raise httpx.HTTPStatusError("err", request=None, response=None)

    class _Client:
        async def __aenter__(self):
            return self

        async def __aexit__(self, *exc):
            return False

        async def post(self, url, json=None, headers=None):
            calls["posts"].append((url, json))
            return _Response(200, {"task_id": submit_task_id})

        async def get(self, url, headers=None):
            calls["gets"] += 1
            status = next(status_iter)
            if status is None:
                return _Response(404, {})
            body = {"status": status, "result": (results or {}).get(status)}
            return _Response(200, body)

    monkeypatch.setattr(httpx, "AsyncClient", lambda **kw: _Client())
    monkeypatch.setattr("temporalio.activity.heartbeat", lambda *a, **k: None)
    return calls


TRAIN_KWARGS = dict(
    tenant_id="t",
    training_job_id="j",
    dataset_path="p",
    base_model="m",
    method="qlora",
    mode="quick",
    hyperparams={},
    gpu_class="a10g",
    llm_config={"api_base_url": "u", "api_key": "k", "model": "m", "max_tokens": 1},
)

RESULT = {"adapter_path": "s3://a", "adapter_size_bytes": 1, "metrics": {}}


@pytest.mark.asyncio
async def test_submits_and_persists_task_id_before_poll(monkeypatch):
    calls = _install_fake_http(
        monkeypatch,
        submit_task_id="task-1",
        statuses=["RUNNING", "COMPLETE"],
        results={"COMPLETE": RESULT},
    )
    db = _FakeDB(existing=None)
    prov = _make_provider(db)

    out = await prov.run_training(**TRAIN_KWARGS)

    assert out["adapter_path"] == "s3://a"
    assert len(calls["posts"]) == 1
    assert calls["posts"][0][1] == {"payload": calls["posts"][0][1]["payload"]}
    assert db.updates, "task id must be persisted before polling"
    _, args = db.updates[0]
    assert args[0] == "train:task-1"


@pytest.mark.asyncio
async def test_reattaches_without_resubmit(monkeypatch):
    calls = _install_fake_http(
        monkeypatch,
        submit_task_id="never-used",
        statuses=["RUNNING", "COMPLETE"],
        results={"COMPLETE": RESULT},
    )
    db = _FakeDB(existing="train:task-9")
    prov = _make_provider(db)

    out = await prov.run_training(**TRAIN_KWARGS)

    assert out["adapter_path"] == "s3://a"
    assert calls["posts"] == []


@pytest.mark.asyncio
async def test_unknown_stored_id_falls_back_to_fresh_submit(monkeypatch):
    calls = _install_fake_http(
        monkeypatch,
        submit_task_id="task-2",
        statuses=[None, "COMPLETE"],
        results={"COMPLETE": RESULT},
    )
    db = _FakeDB(existing="train:fc-modal-leftover")
    prov = _make_provider(db)

    out = await prov.run_training(**TRAIN_KWARGS)

    assert out["adapter_path"] == "s3://a"
    assert len(calls["posts"]) == 1
    assert db.updates[0][1][0] == "train:task-2"


@pytest.mark.asyncio
async def test_unmapped_gpu_class_raises(monkeypatch):
    prov = _make_provider(_FakeDB())

    with pytest.raises(RuntimeError, match="not available on the Beam provider"):
        await prov.run_training(**{**TRAIN_KWARGS, "gpu_class": "a10080gb_dual"})


@pytest.mark.asyncio
async def test_terminal_error_status_raises(monkeypatch):
    _install_fake_http(monkeypatch, submit_task_id="task-3", statuses=["ERROR"])
    prov = _make_provider(_FakeDB())

    with pytest.raises(RuntimeError, match="ended ERROR"):
        await prov.run_training(**TRAIN_KWARGS)


@pytest.mark.asyncio
async def test_on_policy_refused_with_actionable_error(monkeypatch):
    prov = _make_provider(_FakeDB())

    with pytest.raises(RuntimeError, match="gpu_provider='modal'"):
        await prov.run_training(
            **{**TRAIN_KWARGS, "hyperparams": {"distill_method": "on_policy"}}
        )


def test_base64_pickled_result_is_decoded():
    import base64
    import pickle

    blob = base64.b64encode(pickle.dumps(RESULT)).decode()

    assert gp.BeamGpuProvider._decode_result({"base64": blob}) == RESULT


def test_json_string_result_is_decoded():
    assert gp.BeamGpuProvider._decode_result(json.dumps(RESULT)) == RESULT


def test_gpu_class_specific_queue_wins_over_generic():
    prov = _make_provider(
        _FakeDB(),
        queue_urls={"train": "https://generic.test", "train@a10g": "https://a10g.test"},
    )

    assert prov._queue_url("train", "a10g") == "https://a10g.test"
    assert prov._queue_url("train", "l40s") == "https://generic.test"
