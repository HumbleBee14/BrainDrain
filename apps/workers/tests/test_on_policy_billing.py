"""Who pays for the teacher that runs inside an on-policy training container.

The teacher-GPU spend cap is the only thing standing between a tenant and
unbounded teacher time on our own hardware, and it counts billing rows. An
on-policy run's teacher spends its whole life inside a `training` container, so
unless its share is written as its own operation the cap sums a total the run
never contributes to — and every subsequent improve pass is admitted against it.

These tests are about the split: that it happens for on-policy and nothing else,
that the two rows re-add to exactly one container's bill, and that a failed run
is not a free teacher.
"""

import json
import re
import uuid
from dataclasses import dataclass, field
from pathlib import Path

import pytest

from src.activities.train_model import (
    DISTILL_METHOD_HYPERPARAM,
    _append_training_billing_outbox,
    _teacher_reservation_billing_event_id,
    _teacher_serving_billing_event_id,
    _training_billing_event_id,
    split_teacher_serving_cost,
    teacher_serving_share,
)
from src.constants import GPU_DEVICE_COUNTS, ON_POLICY_DISTILL_METHOD

TENANT = "11111111-1111-1111-1111-111111111111"
JOB = "22222222-2222-2222-2222-222222222222"

ON_POLICY_HP = {DISTILL_METHOD_HYPERPARAM: ON_POLICY_DISTILL_METHOD}


@dataclass
class _Row:
    operation: str
    gpu_seconds: int
    cost_usd: float
    metadata: dict


@dataclass
class _FakeConn:
    """Records the outbox rows a billing path appends, keyed by operation.

    Also stands in for the run's admission reservation: `pending_reservation`
    answers the DELETE that retires it, `delivered_reservation_cost` answers the
    lookup for one the relay already delivered.
    """

    rows: list[_Row] = field(default_factory=list)
    pending_reservation: bool = False
    delivered_reservation_cost: float | None = None
    voided: list[uuid.UUID] = field(default_factory=list)

    async def execute(self, sql: str, *args):
        operation = "teacher_serving" if "'teacher_serving'" in sql else "training"
        _id, _tenant, _resource, gpu_seconds, cost_usd, metadata = args
        self.rows.append(_Row(operation, gpu_seconds, cost_usd, json.loads(metadata)))

    async def fetchval(self, sql: str, *args):
        if sql.lstrip().startswith("DELETE"):
            if self.pending_reservation:
                self.pending_reservation = False
                self.voided.append(args[0])
                return args[0]
            return None
        return self.delivered_reservation_cost

    def by_operation(self, operation: str) -> _Row | None:
        return next((row for row in self.rows if row.operation == operation), None)


async def append(conn, *, share: float, gpu_seconds: int = 3600, cost_usd: float = 6.00):
    await _append_training_billing_outbox(
        conn,
        tenant_id=TENANT,
        job_id=JOB,
        outcome="completed",
        gpu_seconds=gpu_seconds,
        cost_usd=cost_usd,
        teacher_share=share,
        metadata={"status": "completed", "gpu_class": "a10080gb_dual"},
    )


def test_only_an_on_policy_run_has_a_teacher_in_its_container():
    assert teacher_serving_share("a10080gb_dual", ON_POLICY_HP) == 0.5
    assert teacher_serving_share("a10080gb_dual", {}) == 0.0
    assert teacher_serving_share("a10080gb_dual", {DISTILL_METHOD_HYPERPARAM: "logit"}) == 0.0


def test_a_single_device_class_has_no_teacher_share_to_split():
    """An on-policy run cannot start on one card, but a mislabelled job must not
    bill half its training time as teacher time on the way to failing."""
    assert teacher_serving_share("a10080gb", ON_POLICY_HP) == 0.0
    assert teacher_serving_share(None, ON_POLICY_HP) == 0.0
    assert teacher_serving_share("A10080GB_DUAL", ON_POLICY_HP) == 0.5


def test_the_two_halves_re_add_to_one_containers_bill():
    """The tenant is charged for the container it used, not for a container plus a
    rounding error. Odd numbers on purpose: an even split of $6.01 has no exact
    halves, and both rows still have to sum to $6.01."""
    (student_seconds, student_cost), (teacher_seconds, teacher_cost) = split_teacher_serving_cost(
        3601, 6.01, 0.5
    )

    assert student_seconds + teacher_seconds == 3601
    assert round(student_cost + teacher_cost, 2) == 6.01


def test_no_share_leaves_the_bill_whole():
    assert split_teacher_serving_cost(3600, 6.00, 0.0) == ((3600, 6.00), (0, 0.0))


@pytest.mark.asyncio
async def test_an_on_policy_run_writes_a_teacher_serving_row_the_cap_can_see():
    conn = _FakeConn()

    await append(conn, share=0.5)

    teacher = conn.by_operation("teacher_serving")
    assert teacher is not None, "the spend cap sums this operation and nothing else writes it"
    assert teacher.cost_usd == 3.00
    assert teacher.gpu_seconds == 1800
    assert teacher.metadata["teacher_device_share"] == 0.5


@pytest.mark.asyncio
async def test_the_training_row_drops_to_the_students_share():
    """Both rows at full cost would double-bill the container."""
    conn = _FakeConn()

    await append(conn, share=0.5)

    student = conn.by_operation("training")
    assert student.cost_usd == 3.00
    assert student.gpu_seconds == 1800


@pytest.mark.asyncio
async def test_every_other_mode_still_writes_exactly_one_row():
    conn = _FakeConn()

    await append(conn, share=0.0)

    assert [row.operation for row in conn.rows] == ["training"]
    assert conn.rows[0].cost_usd == 6.00


@pytest.mark.asyncio
async def test_the_two_rows_have_distinct_ids_so_neither_upserts_the_other():
    """Both are derived from the same job id and outcome. Colliding ids would make
    `ON CONFLICT DO NOTHING` silently drop the second row."""
    seen = []

    class _IdCapturingConn(_FakeConn):
        async def execute(self, sql: str, *args):
            seen.append(args[0])
            await super().execute(sql, *args)

    await append(_IdCapturingConn(), share=0.5)

    assert len(seen) == 2
    assert seen[0] != seen[1]
    assert all(isinstance(row_id, uuid.UUID) for row_id in seen)


@pytest.mark.asyncio
async def test_a_finished_run_retires_its_admission_reservation():
    """The reservation held the teacher's estimated share while the run was in
    flight; the terminal charge written here replaces it, in one transaction."""
    conn = _FakeConn(pending_reservation=True)

    await append(conn, share=0.5)

    assert conn.voided == [_teacher_reservation_billing_event_id(JOB)]
    assert [row.operation for row in conn.rows] == ["training", "teacher_serving"]


@pytest.mark.asyncio
async def test_a_reservation_billed_at_its_estimate_suppresses_the_teacher_row():
    """Once the relay reaped and delivered the reservation, the teacher's time is
    in the ledger at the estimate. A terminal teacher row on top of that would
    charge the same GPU twice."""
    conn = _FakeConn(delivered_reservation_cost=3.00)

    await append(conn, share=0.5)

    assert [row.operation for row in conn.rows] == ["training"]
    assert conn.rows[0].cost_usd == 3.00


@pytest.mark.asyncio
async def test_a_reservation_voided_at_zero_settles_nothing():
    """A reservation delivered at zero was voided for a run that had not started
    yet. If the run then ran anyway, its teacher time is still unbilled and the
    terminal row must be written."""
    conn = _FakeConn(delivered_reservation_cost=0.0)

    await append(conn, share=0.5)

    assert [row.operation for row in conn.rows] == ["training", "teacher_serving"]


ENUMS_RS = Path(__file__).resolve().parents[3] / "crates/shared/src/enums.rs"


def test_the_control_plane_splits_the_same_classes_this_worker_does():
    """The reaper and the cancel path close out runs the worker never finished, and
    bill them by device count too. A class either side does not know about is a
    dual-GPU run billed as one card — invisible to the teacher-GPU cap.

    Read from `GpuClass::device_count`, which the control plane calls directly, so
    a new multi-device variant there fails here until this worker knows it too.
    """
    body = ENUMS_RS.read_text(encoding="utf-8")
    arms = re.search(r"fn device_count\(self\) -> u32 \{\s*match self \{(.*?)\n        \}", body, re.DOTALL)
    assert arms, "could not find GpuClass::device_count in the shared enums"

    multi_device = {}
    for variants, count in re.findall(r"((?:GpuClass::\w+\s*\|?\s*)+)=> (\d+),", arms.group(1)):
        if int(count) < 2:
            continue
        for variant in re.findall(r"GpuClass::(\w+)", variants):
            multi_device[_snake_case(variant)] = int(count)

    assert multi_device == GPU_DEVICE_COUNTS


def _snake_case(variant: str) -> str:
    """Rust variant to its serde `snake_case` rename, as `GpuClass` declares."""
    spaced = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", variant)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", spaced).lower()


def test_the_control_plane_writes_the_ledger_ids_this_worker_writes():
    """Both sides can close out the same run — the reaper when a worker dies, the
    worker if it outlives its own reaping. Matching ids make the second write a
    no-op; diverging ones bill the tenant twice.

    Pinned to literal ids rather than read from the Rust source: the same four
    values are asserted by `serving_cost.rs`'s own test, so each side fails on its
    own drift. Grepping the other language's file would pass on the ids sitting in
    its test fixtures while its production format string had already changed.
    """
    job = "11111111-1111-1111-1111-111111111111"

    assert str(_training_billing_event_id(job, "failed")) == (
        "370e6ec6-c631-542b-921c-1a3e9e462fbc"
    )
    assert str(_teacher_serving_billing_event_id(job, "failed")) == (
        "1f8b1c51-493f-55ec-b240-d742d5ae9a13"
    )
    assert str(_training_billing_event_id(job, "cancelled")) == (
        "e79ce3d1-9ac2-50df-9c9f-ffe652223541"
    )
    assert str(_teacher_serving_billing_event_id(job, "cancelled")) == (
        "fefbc12a-0217-5bb1-987d-dee962b2a406"
    )
    assert str(_teacher_reservation_billing_event_id(job)) == (
        "158bff52-8237-5d12-a293-b3f29f0e2095"
    )
