"""Tests for training worker terminal-state guards and idempotent retry.

Covers the decision points added to StartTrainingActivity without needing a GPU:
- a Temporal retry of an already-completed job returns the persisted result;
- a job cancelled before start is not (re-)run;
- the reconstruction helper only fires for completed jobs with a model row.
"""

import pytest
from temporalio.exceptions import ApplicationError

from src.activities.stubs import StartTrainingInput, StartTrainingOutput
from src.activities.train_model import StartTrainingActivity


class _FakeConn:
    """Connection stand-in used inside the except-path transaction."""

    def __init__(self):
        self.executed = []

    def transaction(self):
        conn = self

        class _Tx:
            async def __aenter__(self):
                return conn

            async def __aexit__(self, *exc):
                return False

        return _Tx()

    async def fetchval(self, query, *args):
        # The guarded FAILED update: a cancelled job matches 0 rows → None.
        return None

    async def execute(self, query, *args):
        self.executed.append(query)


class _FakeDB:
    """Minimal asyncpg-pool stand-in scripting the queries run() issues."""

    def __init__(self, *, started_id, existing_row=None):
        self._started_id = started_id
        self._existing_row = existing_row
        self.fetchval_calls = 0
        self.conn = _FakeConn()

    async def fetchval(self, query, *args):
        # The only db.fetchval on the pool is the guarded start-claim UPDATE.
        self.fetchval_calls += 1
        return self._started_id

    async def fetchrow(self, query, *args):
        # Used by _existing_result_if_completed.
        return self._existing_row

    def acquire(self):
        conn = self.conn

        class _Acquire:
            async def __aenter__(self):
                return conn

            async def __aexit__(self, *exc):
                return False

        return _Acquire()


def _make_activity(db):
    act = StartTrainingActivity.__new__(StartTrainingActivity)
    act.infra = type("Infra", (), {"db": db})()
    act.gpu_provider = None
    return act


def _input():
    return StartTrainingInput(
        tenant_id="11111111-1111-1111-1111-111111111111",
        training_job_id="22222222-2222-2222-2222-222222222222",
        dataset_path="s3://x",
        base_model="Qwen/Qwen2.5-0.5B",
        method="lora",
        mode="quick",
        hyperparams={},
        gpu_class="T4",
    )


@pytest.mark.asyncio
async def test_completed_job_retry_returns_existing_result_without_training():
    existing = {
        "status": "completed",
        "metrics": {"estimated_cost": 1.23},
        "adapter_path": "s3://bucket/tenant/adapters/job/adapter",
        "adapter_size_bytes": 4096,
        "model_id": "33333333-3333-3333-3333-333333333333",
    }
    db = _FakeDB(started_id=None, existing_row=existing)
    act = _make_activity(db)

    result = await act.run(_input())

    assert isinstance(result, StartTrainingOutput)
    assert result.adapter_path == existing["adapter_path"]
    assert result.adapter_size_bytes == 4096
    assert result.metrics == {"estimated_cost": 1.23}


@pytest.mark.asyncio
async def test_cancelled_before_start_is_not_run():
    # started_id None and no completed row → job was cancelled/failed; must raise
    # non-retryably instead of running training.
    db = _FakeDB(started_id=None, existing_row={"status": "cancelled"})
    act = _make_activity(db)

    with pytest.raises(ApplicationError) as exc:
        await act.run(_input())
    assert exc.value.non_retryable is True


@pytest.mark.asyncio
async def test_existing_result_helper_none_when_not_completed():
    db = _FakeDB(started_id=None, existing_row={"status": "training"})
    assert await StartTrainingActivity._existing_result_if_completed(db, "job") is None


@pytest.mark.asyncio
async def test_existing_result_helper_none_when_no_adapter():
    row = {
        "status": "completed",
        "metrics": {},
        "adapter_path": None,
        "adapter_size_bytes": None,
        "model_id": None,
    }
    db = _FakeDB(started_id=None, existing_row=row)
    assert await StartTrainingActivity._existing_result_if_completed(db, "job") is None
