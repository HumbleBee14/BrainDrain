"""The seam between the teacher scoring pass and the student's training run.

Stage 2 is only real if something calls it, and the failure mode of getting this
wrong is silent: a job that paid for a teacher's distributions and then trained
plain SFT reports success either way. So these tests cover the wiring itself —
what gates the extra GPU pass, what every other run still does (nothing new), the
hyperparams the strategy is selected by, the positional argument order Temporal
binds by index, and who writes the extraction's status.
"""

import ast
import asyncio
import inspect
import logging
import re
from datetime import timedelta
from pathlib import Path
from unittest.mock import MagicMock

import pytest
from temporalio.activity import _Definition
from temporalio.exceptions import ApplicationError

from src.activities.extract_logprobs import MANIFEST_NAME, artifact_prefix
from src.activities.pipeline_records import TeacherExtractionStatus
from src.activities.stubs import ExtractTeacherLogprobsOutput, StartTrainingOutput
from src.activities.train_model import _TEACHER_ARTIFACTS_HYPERPARAM, resolve_strategy_key
from src.worker import build_activity_lists
from src.workflows import train as train_wf

REPO_ROOT = Path(__file__).resolve().parents[3]
TEMPORAL_RS = REPO_ROOT / "crates/api/src/temporal.rs"

PLAN = {
    "distill_method": "logit",
    "teacher_model": "Qwen/Qwen3-32B",
    "teacher_revision": "a" * 40,
    "precision": "fp8",
    "top_k_logprobs": 64,
    "gpu_class": "a10080gb",
    "est_cost_usd": 1.25,
    "est_gpu_hours": 0.4,
}


class TestGate:
    @pytest.mark.parametrize(
        "teacher_config",
        [None, {}, {"host": "h", "model": "m"}, {"extraction": None}, {"extraction": "logit"}, "x"],
    )
    def test_no_plan_means_no_extraction(self, teacher_config):
        assert train_wf.extraction_plan(teacher_config) is None

    def test_the_extraction_block_is_the_plan(self):
        assert train_wf.extraction_plan({"model": "m", "extraction": PLAN}) == PLAN


class TestPlanTranslation:
    def test_plan_keys_map_onto_the_activity_input(self):
        input = train_wf.extraction_input(
            PLAN,
            tenant_id="t",
            training_job_id="j",
            dataset_path="tenants/t/datasets/d.jsonl",
            base_model="Qwen/Qwen3-8B",
        )

        assert input.teacher_model == "Qwen/Qwen3-32B"
        assert input.teacher_revision == "a" * 40
        assert input.student_model == "Qwen/Qwen3-8B"
        assert input.precision == "fp8"
        assert input.top_k == 64
        assert input.gpu_class == "a10080gb"
        assert input.dataset_path == "tenants/t/datasets/d.jsonl"

    def test_the_teachers_gpu_class_is_used_not_the_students(self):
        """The plan's gpu_class is what the *teacher* needs to run at all."""
        input = train_wf.extraction_input(
            PLAN,
            tenant_id="t",
            training_job_id="j",
            dataset_path="d.jsonl",
            base_model="Qwen/Qwen3-8B",
        )

        assert input.gpu_class == PLAN["gpu_class"]

    def test_tuning_knobs_fall_back_to_the_activity_defaults(self):
        input = train_wf.extraction_input(
            {"distill_method": "logit", "teacher_model": "Qwen/Qwen3-32B"},
            tenant_id="t",
            training_job_id="j",
            dataset_path="d.jsonl",
            base_model="Qwen/Qwen3-8B",
        )

        assert input.precision == "bf16"
        assert input.top_k == 32
        assert input.teacher_revision == ""
        assert input.gpu_class is None


class TestTrainingHyperparams:
    def test_both_keys_select_and_feed_the_logit_strategy(self):
        hp = train_wf.hyperparams_with_artifacts({"r": 16}, "tenants/t/d-teacher-logprobs/abc/")

        assert hp["r"] == 16
        assert resolve_strategy_key("distill", hp["distill_method"]) == "distill_logit"
        assert hp[_TEACHER_ARTIFACTS_HYPERPARAM] == "tenants/t/d-teacher-logprobs/abc/"

    def test_the_callers_hyperparams_are_not_mutated(self):
        original = {"r": 16}
        train_wf.hyperparams_with_artifacts(original, "p/")

        assert original == {"r": 16}

    def test_the_prefix_is_where_the_strategy_looks_for_the_manifest(self):
        """Both halves must agree that the manifest is at `<prefix>manifest.json`."""
        prefix = artifact_prefix("tenants/t/datasets/d.jsonl", "abc123")
        strategy_source = (
            Path(train_wf.__file__).parents[1] / "activities" / "train_model.py"
        ).read_text()

        assert prefix.endswith("/")
        assert f'_download_dataset(prefix + "{MANIFEST_NAME}"' in strategy_source


class TestUnsupportedPlans:
    def test_a_supported_plan_has_no_reason_to_refuse(self):
        assert train_wf.unsupported_plan_reason(PLAN, "distill") is None

    @pytest.mark.parametrize("mode", ["quick", "iterative", "aligned", "reasoning"])
    def test_fidelity_outside_distill_mode_is_refused(self, mode):
        assert "no meaning" in train_wf.unsupported_plan_reason(PLAN, mode)

    def test_an_unknown_method_is_refused_rather_than_scored(self):
        plan = {**PLAN, "distill_method": "attention"}

        assert "Unsupported" in train_wf.unsupported_plan_reason(plan, "distill")

    def test_a_plan_without_a_teacher_is_refused(self):
        plan = {**PLAN, "teacher_model": ""}

        assert train_wf.unsupported_plan_reason(plan, "distill") is not None


@pytest.fixture
def recorded(monkeypatch):
    """Run TrainWorkflow.run outside Temporal, recording what it dispatches."""
    calls = []

    async def fake_execute_activity(name, arg=None, **kwargs):
        calls.append((name, arg, kwargs))
        if name == "extract_teacher_logprobs":
            return ExtractTeacherLogprobsOutput(
                manifest_path="tenants/t/d-teacher-logprobs/abc/manifest.json",
                artifact_prefix="tenants/t/d-teacher-logprobs/abc/",
                records=10,
                scored_positions=100,
                skipped_records=0,
                shards=1,
                metrics={},
            )
        if name == "start_training":
            return StartTrainingOutput(adapter_path="a", adapter_size_bytes=1, metrics={})
        return None

    async def fake_execute_child_workflow(run, **kwargs):
        calls.append(("child", kwargs.get("args"), kwargs))
        return StartTrainingOutput(adapter_path="a", adapter_size_bytes=1, metrics={})

    monkeypatch.setattr(train_wf.workflow, "execute_activity", fake_execute_activity)
    monkeypatch.setattr(train_wf.workflow, "execute_child_workflow", fake_execute_child_workflow)
    monkeypatch.setattr(train_wf.workflow, "set_current_details", lambda _detail: None)
    monkeypatch.setattr(train_wf.workflow, "logger", logging.getLogger("test.train"))
    for timeout in (
        "teacher_extraction_activity",
        "teacher_extraction_heartbeat",
        "train_activity",
        "train_heartbeat",
        "db_lookup",
    ):
        monkeypatch.setattr(train_wf.timeouts, timeout, lambda: timedelta(minutes=1))
    return calls


def run_train(*, mode="distill", teacher_config=None, hyperparams=None):
    return asyncio.run(
        train_wf.TrainWorkflow().run(
            "tenant",
            "job",
            "tenants/t/datasets/d.jsonl",
            "Qwen/Qwen3-8B",
            "qlora",
            mode,
            hyperparams if hyperparams is not None else {"r": 16},
            "t4",
            teacher_config,
        )
    )


class TestWorkflowDispatch:
    @pytest.mark.parametrize("mode", ["quick", "distill", "iterative", "aligned", "reasoning"])
    def test_a_job_without_a_plan_scores_nothing(self, recorded, mode):
        """The regression guard: every pre-Stage-2 run must be untouched."""
        run_train(mode=mode)

        teacher_calls = [
            name for name, _, _ in recorded if name.startswith(("extract", "set_teacher"))
        ]
        assert teacher_calls == []

    @pytest.mark.parametrize("mode", ["quick", "distill"])
    def test_hyperparams_reach_training_unchanged_without_a_plan(self, recorded, mode):
        run_train(mode=mode)

        (_, arg, _) = next(call for call in recorded if call[0] == "start_training")
        assert arg.hyperparams == {"r": 16}

    def test_a_plan_scores_before_training_and_pins_the_gpu_queue(self, recorded):
        run_train(teacher_config={"model": "Qwen/Qwen3-32B", "extraction": PLAN})

        names = [name for name, _, _ in recorded]
        assert names == [
            "set_teacher_extraction_status",
            "extract_teacher_logprobs",
            "set_teacher_extraction_status",
            "start_training",
        ]
        extract = next(call for call in recorded if call[0] == "extract_teacher_logprobs")
        assert extract[2]["task_queue"] == "ml-pipeline-gpu"

    def test_training_is_handed_the_scored_artifacts(self, recorded):
        run_train(teacher_config={"extraction": PLAN})

        (_, arg, _) = next(call for call in recorded if call[0] == "start_training")
        assert arg.hyperparams["distill_method"] == "logit"
        assert arg.hyperparams[_TEACHER_ARTIFACTS_HYPERPARAM] == (
            "tenants/t/d-teacher-logprobs/abc/"
        )
        assert arg.hyperparams["r"] == 16

    @pytest.mark.parametrize(
        "hyperparams",
        [
            {"distill_method": "logit"},
            {"teacher_artifacts_prefix": "tenants/someone-else/d-teacher-logprobs/abc/"},
        ],
    )
    def test_caller_supplied_fidelity_keys_are_refused(self, recorded, hyperparams):
        """Only an admitted plan may select the logit strategy or name its prefix."""
        with pytest.raises(ApplicationError) as excinfo:
            run_train(hyperparams={"r": 16, **hyperparams})

        assert excinfo.value.non_retryable
        assert [name for name, _, _ in recorded] == []

    def test_a_plan_on_a_non_distill_mode_refuses_before_the_gpu(self, recorded):
        with pytest.raises(ApplicationError) as excinfo:
            run_train(mode="quick", teacher_config={"extraction": PLAN})

        assert excinfo.value.non_retryable
        assert [name for name, _, _ in recorded] == []


class TestExtractionStatus:
    def test_running_then_completed_around_a_successful_pass(self, recorded):
        run_train(teacher_config={"extraction": PLAN})

        statuses = [
            arg.status for name, arg, _ in recorded if name == "set_teacher_extraction_status"
        ]
        assert statuses == [TeacherExtractionStatus.RUNNING, TeacherExtractionStatus.COMPLETED]

    def test_the_completed_transition_carries_what_the_pass_measured(self, monkeypatch, recorded):
        """The terminal write is what bills the GPU, so it has to be handed the
        runtimes — without them the charge falls back to the estimate."""

        async def measured_activity(name, arg=None, **kwargs):
            recorded.append((name, arg, kwargs))
            if name == "extract_teacher_logprobs":
                return ExtractTeacherLogprobsOutput(
                    manifest_path="p/manifest.json",
                    artifact_prefix="p/",
                    records=1,
                    scored_positions=1,
                    skipped_records=0,
                    shards=1,
                    metrics={"teacher_load_seconds": 30.0, "scoring_seconds": 90.0},
                )
            if name == "start_training":
                return StartTrainingOutput(adapter_path="a", adapter_size_bytes=1, metrics={})
            return None

        monkeypatch.setattr(train_wf.workflow, "execute_activity", measured_activity)
        run_train(teacher_config={"extraction": PLAN})

        statuses = [arg for name, arg, _ in recorded if name == "set_teacher_extraction_status"]
        assert statuses[0].metrics is None
        assert statuses[1].metrics == {"teacher_load_seconds": 30.0, "scoring_seconds": 90.0}

    def test_a_failed_pass_reports_no_runtime_to_bill_from(self, monkeypatch, recorded):
        async def failing_activity(name, arg=None, **kwargs):
            recorded.append((name, arg, kwargs))
            if name == "extract_teacher_logprobs":
                raise ApplicationError("teacher OOM")
            return None

        monkeypatch.setattr(train_wf.workflow, "execute_activity", failing_activity)

        with pytest.raises(ApplicationError):
            run_train(teacher_config={"extraction": PLAN})

        statuses = [arg for name, arg, _ in recorded if name == "set_teacher_extraction_status"]
        assert [status.metrics for status in statuses] == [None, None]

    def test_the_status_write_is_tenant_scoped(self, recorded):
        run_train(teacher_config={"extraction": PLAN})

        (_, arg, _) = next(call for call in recorded if call[0] == "set_teacher_extraction_status")
        assert arg.tenant_id == "tenant"
        assert arg.training_job_id == "job"

    def test_a_failed_pass_is_recorded_and_training_never_starts(self, monkeypatch, recorded):
        async def failing_activity(name, arg=None, **kwargs):
            recorded.append((name, arg, kwargs))
            if name == "extract_teacher_logprobs":
                raise ApplicationError("teacher OOM")
            return None

        monkeypatch.setattr(train_wf.workflow, "execute_activity", failing_activity)

        with pytest.raises(ApplicationError):
            run_train(teacher_config={"extraction": PLAN})

        names = [name for name, _, _ in recorded]
        statuses = [
            arg.status for name, arg, _ in recorded if name == "set_teacher_extraction_status"
        ]
        assert statuses == [TeacherExtractionStatus.RUNNING, TeacherExtractionStatus.FAILED]
        assert "start_training" not in names

    def test_cancellation_leaves_the_status_running(self, monkeypatch, recorded):
        """A cancelled extraction must stay distinguishable from a failed one."""

        async def cancelled_activity(name, arg=None, **kwargs):
            recorded.append((name, arg, kwargs))
            if name == "extract_teacher_logprobs":
                raise asyncio.CancelledError()
            return None

        monkeypatch.setattr(train_wf.workflow, "execute_activity", cancelled_activity)

        with pytest.raises(asyncio.CancelledError):
            run_train(teacher_config={"extraction": PLAN})

        statuses = [
            arg.status for name, arg, _ in recorded if name == "set_teacher_extraction_status"
        ]
        assert statuses == [TeacherExtractionStatus.RUNNING]


def rust_train_argument_order() -> list[str]:
    """The positional array the API sends to TrainWorkflow, in order."""
    source = TEMPORAL_RS.read_text()
    body = re.search(r"fn train_workflow_args\(.*?json!\(\[(.*?)\]\)", source, re.S)
    assert body, f"expected train_workflow_args in {TEMPORAL_RS}"
    return [
        re.sub(r"\..*$", "", line.strip().rstrip(","))
        for line in body.group(1).strip().splitlines()
    ]


class TestPayloadCompatibility:
    def test_python_binds_the_arguments_the_api_sends(self):
        params = list(inspect.signature(train_wf.TrainWorkflow.run).parameters)[1:]

        assert params == rust_train_argument_order()

    def test_teacher_config_is_last_and_optional(self):
        params = inspect.signature(train_wf.TrainWorkflow.run).parameters

        assert list(params)[-1] == "teacher_config"
        assert params["teacher_config"].default is None

    def test_a_payload_sent_before_teacher_config_existed_still_binds(self):
        """Old in-flight payloads carry eight arguments; none may shift position."""
        bound = inspect.signature(train_wf.TrainWorkflow.run).bind(
            MagicMock(),
            "tenant",
            "job",
            "d.jsonl",
            "Qwen/Qwen3-8B",
            "qlora",
            "quick",
            {"r": 16},
            "t4",
        )
        bound.apply_defaults()

        assert bound.arguments["mode"] == "quick"
        assert bound.arguments["gpu_class"] == "t4"
        assert bound.arguments["teacher_config"] is None


class TestRegistration:
    def test_extraction_runs_on_the_gpu_queue_and_status_writes_do_not(self):
        cpu, gpu = build_activity_lists(MagicMock(), MagicMock())

        cpu_names = {_Definition.must_from_callable(c).name for c in cpu}
        gpu_names = {_Definition.must_from_callable(c).name for c in gpu}
        assert "extract_teacher_logprobs" in gpu_names
        assert "set_teacher_extraction_status" in cpu_names

    def test_the_workflow_asks_for_the_registered_activity_names(self):
        tree = ast.parse(Path(train_wf.__file__).read_text())
        called = {
            node.args[0].value
            for node in ast.walk(tree)
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and node.func.attr == "execute_activity"
            and node.args
            and isinstance(node.args[0], ast.Constant)
        }
        cpu, gpu = build_activity_lists(MagicMock(), MagicMock())
        registered = {_Definition.must_from_callable(c).name for c in cpu + gpu}

        assert called <= registered
