"""What the teacher's GPU pass costs, and that somebody records it.

The API refuses an extraction whose estimate would push a tenant over its
monthly teacher-GPU cap, and it sums that spend from `extraction` billing rows.
For as long as nothing wrote one, the sum was always zero and the cap could not
trip — so these tests are about the write, not the arithmetic: that a finished
pass produces exactly one row at the measured cost, that an abandoned one leaves
a reapable row at the admitted estimate rather than nothing, and that no ordering
of retries charges twice.
"""

import asyncio
import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from types import SimpleNamespace

import pytest

from src.activities.pipeline_records import (
    SetTeacherExtractionStatusActivity,
    SetTeacherExtractionStatusInput,
    TeacherExtractionStatus,
    extraction_billing_event_id,
    extraction_charge,
    measured_extraction_seconds,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
BILLING_OUTBOX_RS = REPO_ROOT / "crates/api/src/services/billing_outbox.rs"
TEACHER_CAP_RS = REPO_ROOT / "crates/api/src/services/teacher/billing.rs"
ENUMS_RS = REPO_ROOT / "crates/shared/src/enums.rs"

# Every place a worker appends a billing_outbox row for teacher GPU time.
WORKER_BILLING_SOURCES = (
    Path(__file__).resolve().parents[1] / "src/activities/pipeline_records.py",
    Path(__file__).resolve().parents[1] / "src/activities/train_model.py",
)


def rust_teacher_gpu_operations() -> list[str]:
    """Wire names of the operations the teacher-GPU cap sums, read from the enum."""
    body = ENUMS_RS.read_text(encoding="utf-8")
    listed = re.search(r"fn teacher_gpu_operations\(\)[^{]*\{\s*\[(.*?)\]", body, re.DOTALL)
    assert listed, "could not find teacher_gpu_operations() in the shared enums"

    variants = re.findall(r"BillingOperation::(\w+)", listed.group(1))
    assert variants, "teacher_gpu_operations() listed no operations"
    return [re.sub(r"(?<!^)(?=[A-Z])", "_", variant).lower() for variant in variants]

TENANT = "11111111-1111-1111-1111-111111111111"
JOB = "22222222-2222-2222-2222-222222222222"
OTHER_TENANT = "33333333-3333-3333-3333-333333333333"

# What the API admitted: 0.5 GPU hours on an $3.00/hr card, quoted at $1.60.
PLAN_CONTEXT = {"est_cost_usd": 1.60, "est_gpu_hours": 0.5, "gpu_class": "a10080gb"}

# 30 minutes of GPU time reported by a pass that finished: $1.50 at $3.00/hr.
MEASURED_METRICS = {"teacher_load_seconds": 120.0, "scoring_seconds": 1680.0}
MEASURED_COST = 1.50


@dataclass
class _OutboxRow:
    tenant_id: str
    resource_id: str
    gpu_seconds: int
    cost_usd: float
    metadata: dict
    delivered: bool = False


@dataclass
class _FakeConn:
    """The three billing_outbox rules the charge path relies on, and no more.

    A primary key on `id`, an upsert that declines a row another tenant owns or
    the relay already delivered, and a jsonb merge on metadata. Everything else
    is asserted on the SQL text rather than simulated.
    """

    context: dict | None
    age_seconds: float | None = None
    rows: dict = field(default_factory=dict)
    job_updates: list = field(default_factory=list)
    queries: list = field(default_factory=list)

    def transaction(self):
        conn = self

        class _Tx:
            async def __aenter__(self):
                return conn

            async def __aexit__(self, *exc):
                return False

        return _Tx()

    async def fetchrow(self, query, *args):
        self.queries.append(query)
        assert "FROM training_jobs" in query
        if self.context is None:
            return None
        reserved = self.rows.get(args[2])
        return {
            **self.context,
            "reserved_seconds_ago": self.age_seconds if reserved is not None else None,
        }

    async def execute(self, query, *args):
        self.queries.append(query)
        if "INSERT INTO billing_outbox" in query:
            assert "DO NOTHING" in query
            self.rows.setdefault(args[0], _row_from(args))
            return "INSERT 0 1"
        assert "UPDATE training_jobs" in query
        self.job_updates.append({"status": args[2], "cost": args[3]})
        return "UPDATE 1"

    async def fetchval(self, query, *args):
        self.queries.append(query)
        if "INSERT INTO billing_outbox" in query:
            assert "DO UPDATE" in query
            existing = self.rows.get(args[0])
            if existing is None:
                self.rows[args[0]] = _row_from(args)
                return args[4]
            if existing.delivered or existing.tenant_id != args[1]:
                return None
            existing.gpu_seconds = args[3]
            existing.cost_usd = args[4]
            existing.metadata = {**existing.metadata, **json.loads(args[5])}
            return existing.cost_usd
        assert "SELECT cost_usd FROM billing_outbox" in query
        row = self.rows.get(args[0])
        return None if row is None or row.tenant_id != args[1] else row.cost_usd


def _row_from(args) -> _OutboxRow:
    return _OutboxRow(
        tenant_id=args[1],
        resource_id=args[2],
        gpu_seconds=args[3],
        cost_usd=args[4],
        metadata=json.loads(args[5]),
    )


class _FakeDB:
    def __init__(self, conn):
        self.conn = conn

    def acquire(self):
        conn = self.conn

        class _Acquire:
            async def __aenter__(self):
                return conn

            async def __aexit__(self, *exc):
                return False

        return _Acquire()


def _activity(conn, *, min_billable_seconds=300):
    act = SetTeacherExtractionStatusActivity.__new__(SetTeacherExtractionStatusActivity)
    act.infra = SimpleNamespace(
        db=_FakeDB(conn),
        settings=SimpleNamespace(min_billable_seconds=min_billable_seconds),
    )
    return act


def _set_status(conn, status, *, metrics=None, tenant_id=TENANT, min_billable_seconds=300):
    asyncio.run(
        _activity(conn, min_billable_seconds=min_billable_seconds).run(
            SetTeacherExtractionStatusInput(
                tenant_id=tenant_id,
                training_job_id=JOB,
                status=status,
                metrics=metrics,
            )
        )
    )


def _only_row(conn) -> _OutboxRow:
    assert len(conn.rows) == 1, f"expected exactly one billing row, got {len(conn.rows)}"
    return next(iter(conn.rows.values()))


class TestReservation:
    def test_starting_the_pass_reserves_one_row_at_the_admitted_estimate(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT))

        _set_status(conn, TeacherExtractionStatus.RUNNING)

        row = _only_row(conn)
        assert row.cost_usd == 1.60
        assert row.gpu_seconds == 1800
        assert row.tenant_id == TENANT
        assert row.resource_id == JOB
        assert row.metadata["extraction_pending"] is True

    def test_the_reservation_is_billed_as_the_operation_the_cap_sums(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT))

        _set_status(conn, TeacherExtractionStatus.RUNNING)

        inserts = [q for q in conn.queries if "INSERT INTO billing_outbox" in q]
        assert inserts and all("'extraction'" in q for q in inserts)

    def test_nothing_is_stamped_on_the_job_before_the_pass_ends(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT))

        _set_status(conn, TeacherExtractionStatus.RUNNING)

        assert conn.job_updates == [{"status": TeacherExtractionStatus.RUNNING, "cost": None}]

    def test_a_retried_start_does_not_reserve_twice(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT))

        _set_status(conn, TeacherExtractionStatus.RUNNING)
        _set_status(conn, TeacherExtractionStatus.RUNNING)

        assert _only_row(conn).cost_usd == 1.60

    def test_a_crash_after_the_reservation_leaves_a_reapable_row_not_nothing(self):
        """The crash point the reservation exists for: control is lost between
        reserving and finalizing, and the estimate must already be on disk."""
        conn = _FakeConn(context=dict(PLAN_CONTEXT))

        _set_status(conn, TeacherExtractionStatus.RUNNING)

        row = _only_row(conn)
        assert row.metadata["extraction_pending"] is True
        assert row.cost_usd == PLAN_CONTEXT["est_cost_usd"]

    def test_a_job_belonging_to_another_tenant_is_never_charged(self):
        conn = _FakeConn(context=None)

        _set_status(conn, TeacherExtractionStatus.RUNNING, tenant_id=OTHER_TENANT)

        assert conn.rows == {}
        assert conn.job_updates == []


class TestFinalization:
    def test_a_completed_pass_bills_the_gpu_time_it_measured(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT))
        _set_status(conn, TeacherExtractionStatus.RUNNING)

        _set_status(conn, TeacherExtractionStatus.COMPLETED, metrics=MEASURED_METRICS)

        row = _only_row(conn)
        assert row.cost_usd == MEASURED_COST
        assert row.gpu_seconds == 1800
        assert row.metadata["extraction_pending"] is False
        assert row.metadata["status"] == TeacherExtractionStatus.COMPLETED

    def test_the_finalized_figure_lands_on_the_job_row(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT))
        _set_status(conn, TeacherExtractionStatus.RUNNING)

        _set_status(conn, TeacherExtractionStatus.COMPLETED, metrics=MEASURED_METRICS)

        assert conn.job_updates[-1] == {
            "status": TeacherExtractionStatus.COMPLETED,
            "cost": MEASURED_COST,
        }

    def test_reusing_a_previous_runs_artifacts_costs_nothing(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT))
        _set_status(conn, TeacherExtractionStatus.RUNNING)

        _set_status(conn, TeacherExtractionStatus.COMPLETED, metrics={"reused_artifacts": 1})

        assert _only_row(conn).cost_usd == 0.0
        assert conn.job_updates[-1]["cost"] == 0.0

    def test_a_retried_finalization_does_not_charge_twice(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT))
        _set_status(conn, TeacherExtractionStatus.RUNNING)

        _set_status(conn, TeacherExtractionStatus.COMPLETED, metrics=MEASURED_METRICS)
        _set_status(conn, TeacherExtractionStatus.COMPLETED, metrics=MEASURED_METRICS)

        assert _only_row(conn).cost_usd == MEASURED_COST

    def test_a_charge_already_in_the_ledger_is_left_alone(self):
        """Once the relay has delivered the reaped estimate, rewriting the outbox
        row would only make the buffer disagree with the ledger."""
        conn = _FakeConn(context=dict(PLAN_CONTEXT))
        _set_status(conn, TeacherExtractionStatus.RUNNING)
        _only_row(conn).delivered = True

        _set_status(conn, TeacherExtractionStatus.COMPLETED, metrics=MEASURED_METRICS)

        assert _only_row(conn).cost_usd == 1.60
        assert conn.job_updates[-1]["cost"] == 1.60

    def test_a_pass_whose_reservation_was_lost_is_still_charged(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT))

        _set_status(conn, TeacherExtractionStatus.COMPLETED, metrics=MEASURED_METRICS)

        assert _only_row(conn).cost_usd == MEASURED_COST


class TestFailedPass:
    def test_a_failure_is_billed_for_how_long_its_reservation_stood(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT), age_seconds=1800.0)
        _set_status(conn, TeacherExtractionStatus.RUNNING)

        _set_status(conn, TeacherExtractionStatus.FAILED)

        row = _only_row(conn)
        assert row.gpu_seconds == 1800
        assert row.cost_usd == 1.50

    def test_a_failure_too_short_to_be_a_gpu_session_is_voided(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT), age_seconds=60.0)
        _set_status(conn, TeacherExtractionStatus.RUNNING)

        _set_status(conn, TeacherExtractionStatus.FAILED)

        assert _only_row(conn).cost_usd == 0.0

    def test_a_failure_with_no_reservation_to_measure_falls_back_to_the_estimate(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT))

        _set_status(conn, TeacherExtractionStatus.FAILED)

        assert _only_row(conn).cost_usd == 1.60


class TestTenantScoping:
    def test_every_query_the_charge_path_runs_names_the_tenant(self):
        conn = _FakeConn(context=dict(PLAN_CONTEXT), age_seconds=900.0)
        _set_status(conn, TeacherExtractionStatus.RUNNING)
        _set_status(conn, TeacherExtractionStatus.COMPLETED, metrics=MEASURED_METRICS)

        assert conn.queries
        for query in conn.queries:
            assert "tenant_id" in query, query


class TestChargeArithmetic:
    def test_measured_runtimes_are_summed_and_nothing_else_is(self):
        seconds = measured_extraction_seconds(
            {"teacher_load_seconds": 10, "scoring_seconds": 5, "shards": 900, "reused": True}
        )

        assert seconds == 15.0

    @pytest.mark.parametrize("metrics", [{}, {"reused_artifacts": 1}, {"shards": 3}])
    def test_a_pass_reporting_no_runtime_costs_nothing(self, metrics):
        assert measured_extraction_seconds(metrics) == 0.0

    def test_measured_runtime_wins_over_the_reservations_age(self):
        charge = extraction_charge(
            metrics=MEASURED_METRICS,
            elapsed_seconds=99_999.0,
            gpu_class="a10080gb",
            estimate=(1800, 1.60),
            min_billable_seconds=300,
        )

        assert charge == (1800, MEASURED_COST)

    def test_an_unknown_gpu_class_falls_back_to_the_default_rate(self):
        _, cost = extraction_charge(
            metrics={"scoring_seconds": 3600.0},
            elapsed_seconds=None,
            gpu_class=None,
            estimate=(0, 0.0),
            min_billable_seconds=300,
        )

        assert cost == 0.80

    def test_the_estimate_is_the_last_resort(self):
        assert extraction_charge(
            metrics=None,
            elapsed_seconds=None,
            gpu_class="a10080gb",
            estimate=(1800, 1.60),
            min_billable_seconds=300,
        ) == (1800, 1.60)


class TestLedgerId:
    def test_the_id_is_derived_from_the_job_and_is_stable(self):
        assert extraction_billing_event_id(JOB) == extraction_billing_event_id(JOB)
        assert extraction_billing_event_id(JOB) != extraction_billing_event_id(TENANT)

    def test_the_id_does_not_collide_with_the_training_charge_for_the_same_job(self):
        from src.activities.train_model import _training_billing_event_id

        assert extraction_billing_event_id(JOB) != _training_billing_event_id(JOB, "completed")


class TestRelayContract:
    """The half of the reservation pattern that lives in the API, in Rust.

    The worker's pending row only behaves as a reservation because the relay
    withholds it from the ledger and reaps it at the fallback. If either side
    renames the flag, extraction silently becomes either unbilled or
    billed-before-it-finishes.
    """

    def test_the_relay_withholds_pending_extraction_rows_from_the_ledger(self):
        source = BILLING_OUTBOX_RS.read_text()
        delivery_filter = re.search(r"FROM billing_outbox.*?FOR UPDATE SKIP LOCKED", source, re.S)

        assert delivery_filter
        assert "extraction_pending" in delivery_filter.group(0)

    def test_the_relay_reaps_an_abandoned_reservation_at_its_fallback(self):
        source = BILLING_OUTBOX_RS.read_text()

        assert "reap_stale_pending_extractions(tx).await?" in source
        assert "'{extraction_reaped}', 'true'::jsonb" in source

    def test_the_cap_sums_every_operation_a_worker_writes(self):
        """The cap is only a cap over spend somebody records. An operation on the
        Rust list that no worker writes makes the cap blind to that spend — which is
        what happened to on-policy: `teacher_serving` was declared and summed while
        the worker billed its whole container as `training`.
        """
        cap = TEACHER_CAP_RS.read_text()
        assert "BillingOperation::teacher_gpu_operations()" in cap, (
            "the cap must sum the declared list, not one hardcoded operation"
        )

        written = "".join(path.read_text() for path in WORKER_BILLING_SOURCES)
        for operation in rust_teacher_gpu_operations():
            assert f"'{operation}'" in written, (
                f"the cap sums '{operation}' but no worker writes an outbox row for it"
            )
