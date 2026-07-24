"""Tests for one-click pipeline row provisioning and id threading."""

import asyncio
from types import SimpleNamespace

from src.activities.build_dataset import BuildDatasetOutput
from src.activities.pipeline_records import (
    CreateEvaluationActivity,
    CreateEvaluationInput,
    CreateTrainingJobActivity,
    CreateTrainingJobInput,
)
from src.activities.stubs import StartTrainingOutput


class _FakeDb:
    def __init__(self, returning):
        self.returning = returning
        self.calls = []

    async def fetchval(self, query, *args):
        self.calls.append((query, args))
        return self.returning


def _infra(db):
    return SimpleNamespace(db=db)


class TestCreateTrainingJob:
    def test_inserts_and_returns_id(self):
        db = _FakeDb(returning="11111111-2222-3333-4444-555555555555")
        activity = CreateTrainingJobActivity(_infra(db))
        job_id = asyncio.run(
            activity.run(
                CreateTrainingJobInput(
                    tenant_id="t1",
                    project_id="p1",
                    dataset_id="d1",
                    base_model="m",
                    method="qlora",
                    mode="quick",
                    hyperparams={"lr": 1e-4},
                    gpu_class="t4",
                )
            )
        )
        assert job_id == "11111111-2222-3333-4444-555555555555"
        query, args = db.calls[0]
        assert "INSERT INTO training_jobs" in query
        assert "RETURNING id" in query
        assert args[0] == "t1" and args[2] == "d1"

    def test_input_defaults_are_back_compatible(self):
        # Constructible without hyperparams/gpu_class.
        inp = CreateTrainingJobInput(
            tenant_id="t",
            project_id="p",
            dataset_id="d",
            base_model="m",
            method="qlora",
            mode="quick",
        )
        assert inp.hyperparams == {} and inp.gpu_class is None


class TestCreateEvaluation:
    def test_inserts_running_row_and_returns_id(self):
        db = _FakeDb(returning="eval-id")
        activity = CreateEvaluationActivity(_infra(db))
        eval_id = asyncio.run(activity.run(CreateEvaluationInput(tenant_id="t1", model_id="m1")))
        assert eval_id == "eval-id"
        query, args = db.calls[0]
        assert "INSERT INTO evaluations" in query
        assert "'running'" in query
        assert args == ("t1", "m1")


class TestOutputBackCompat:
    def test_start_training_output_defaults_model_id(self):
        # Payloads serialized before the field existed must still deserialize.
        out = StartTrainingOutput(adapter_path="a", adapter_size_bytes=1, metrics={})
        assert out.model_id == ""

    def test_build_dataset_output_defaults_dataset_id(self):
        out = BuildDatasetOutput(pair_count=0, storage_path="")
        assert out.dataset_id == ""
