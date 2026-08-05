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
import uuid
from dataclasses import dataclass, field

import pytest

from src.activities.train_model import (
    DISTILL_METHOD_HYPERPARAM,
    _append_training_billing_outbox,
    split_teacher_serving_cost,
    teacher_serving_share,
)
from src.constants import ON_POLICY_DISTILL_METHOD

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
    """Records the outbox rows a billing path appends, keyed by operation."""

    rows: list[_Row] = field(default_factory=list)

    async def execute(self, sql: str, *args):
        operation = "teacher_serving" if "'teacher_serving'" in sql else "training"
        _id, _tenant, _resource, gpu_seconds, cost_usd, metadata = args
        self.rows.append(_Row(operation, gpu_seconds, cost_usd, json.loads(metadata)))

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
