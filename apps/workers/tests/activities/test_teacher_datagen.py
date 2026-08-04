"""Teacher-driven data generation: routing, run-key scoping, provenance."""

import json
from types import SimpleNamespace

import pytest
from temporalio.exceptions import ApplicationError
from temporalio.testing import ActivityEnvironment

from src.activities.build_dataset import BuildDatasetActivity, BuildDatasetInput
from src.activities.generate_pairs import (
    GeneratePairsActivity,
    GenerateSyntheticPairsInput,
    scope_run_key,
)
from src.backends import llm_provider
from src.datagen.impls import LlmPairGenerator
from src.secret_cipher import encrypt_secret
from src.teacher import TeacherConfig

KEY_B64 = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="
TEACHER_MODEL = "big-teacher-72b"
TENANT_MODEL = "tenant-model"
TEACHER_KEY = "sk-teacher-plain"
TENANT_KEY = "sk-tenant-plain"

PAIRS_JSON = json.dumps(
    {
        "generated_qna_pairs": [
            {
                "query": "What does the document say about chunk zero?",
                "answer": ("The document explains chunk zero in detail with grounded content."),
            }
        ]
    }
)
FAITHFUL_JSON = json.dumps({"consistent": True, "score": 0.9, "reason": "grounded"})


class _Body:
    def __init__(self, data: bytes):
        self._data = data

    def read(self) -> bytes:
        return self._data


class FakeS3:
    def __init__(self, objects: dict[str, bytes] | None = None):
        self.store: dict[str, bytes] = dict(objects or {})

    def get_object(self, *, Bucket, Key):
        return {"Body": _Body(self.store[Key])}

    def put_object(self, *, Bucket, Key, Body, ContentType=None):
        self.store[Key] = Body


class FakeDB:
    """Captures execute() calls; fetchrow serves tenant settings."""

    def __init__(self, tenant_settings: dict | None = None):
        self.tenant_settings = tenant_settings
        self.executed: list[tuple[str, tuple]] = []

    async def fetchrow(self, query, *args):
        return {"settings": self.tenant_settings}

    async def execute(self, query, *args):
        self.executed.append((query, args))


class FakeBreaker:
    async def call(self, fn, *args, **kwargs):
        return await fn(*args, **kwargs)


class RoutingProvider:
    """Answers by model id and records every call for routing assertions."""

    def __init__(self):
        self.calls: list[dict] = []

    async def generate(self, http, prompt, **kwargs):
        self.calls.append({"prompt": prompt, **kwargs})
        if kwargs["model"] == TEACHER_MODEL:
            return PAIRS_JSON
        return FAITHFUL_JSON


@pytest.fixture()
def routing_provider():
    provider = RoutingProvider()
    llm_provider.register("teacher_datagen_test", lambda: provider)
    yield provider
    del llm_provider._REGISTRY["teacher_datagen_test"]


def _settings(**overrides):
    values = {
        "llm_provider_backend": "teacher_datagen_test",
        "llm_api_base_url": "https://default.example/v1",
        "llm_api_key": "",
        "llm_model": "default-model",
        "llm_max_tokens": 2000,
        "settings_encryption_key": KEY_B64,
        "generation_temperature": 0.7,
        "judge_temperature": 0.0,
        "datagen_pair_backend": "llm",
        "datagen_facet_backend": "llm",
        "datagen_faithfulness_backend": "llm",
        "faithfulness_gate_enabled": True,
        "pair_checkpoint_enabled": False,
        "url_guard_enabled": False,
        "environment": "development",
    }
    values.update(overrides)
    return SimpleNamespace(**values)


def _teacher_block(**overrides):
    block = {
        "api_base_url": "https://teacher.example.com/v1",
        "model": TEACHER_MODEL,
        "api_key": encrypt_secret(TEACHER_KEY, KEY_B64),
        "policy": "allowed",
        "include_cot": False,
    }
    block.update(overrides)
    return block


def _infra(settings, s3, db):
    return SimpleNamespace(
        s3=s3,
        s3_bucket="bucket",
        settings=settings,
        db=db,
        circuit_breaker=FakeBreaker(),
    )


def _chunks_jsonl(n: int = 1) -> bytes:
    lines = [
        json.dumps({"text": f"chunk-{i} " + "x" * 80, "doc_id": f"d{i}", "chunk_id": f"c{i}"})
        for i in range(n)
    ]
    return "\n".join(lines).encode("utf-8")


def _input(**overrides):
    values = {
        "tenant_id": "t1",
        "project_id": "p1",
        "chunks_storage_path": "chunks.jsonl",
        "task_type": "question_answering",
        "golden_holdout_ratio": 0.0,
        "teacher": _teacher_block(),
    }
    values.update(overrides)
    return GenerateSyntheticPairsInput(**values)


TENANT_SETTINGS = {
    "llm": {
        "api_key": TENANT_KEY,
        "api_base_url": "https://tenant.example.com/v1",
        "model": TENANT_MODEL,
        "max_tokens": 500,
    }
}


class TestTeacherRouting:
    @pytest.mark.asyncio
    async def test_teacher_writes_answers_judge_stays_tenant(self, routing_provider):
        settings = _settings()
        activity = GeneratePairsActivity(
            _infra(settings, FakeS3({"chunks.jsonl": _chunks_jsonl()}), FakeDB(TENANT_SETTINGS))
        )
        env = ActivityEnvironment()
        output = await env.run(activity.run, _input())

        assert output.pair_count == 1
        generation_calls = [c for c in routing_provider.calls if c["model"] == TEACHER_MODEL]
        judge_calls = [c for c in routing_provider.calls if c["model"] == TENANT_MODEL]
        assert generation_calls and judge_calls

        for call in generation_calls:
            assert call["api_base_url"] == "https://teacher.example.com/v1"
            assert call["api_key"] == TEACHER_KEY
        for call in judge_calls:
            assert call["api_base_url"] == "https://tenant.example.com/v1"
            assert call["api_key"] == TENANT_KEY

    @pytest.mark.asyncio
    async def test_no_teacher_keeps_tenant_generation(self, routing_provider):
        settings = _settings(faithfulness_gate_enabled=False)
        activity = GeneratePairsActivity(
            _infra(settings, FakeS3({"chunks.jsonl": _chunks_jsonl()}), FakeDB(TENANT_SETTINGS))
        )
        env = ActivityEnvironment()

        # Tenant model answers PAIRS_JSON only for the teacher model id, so
        # route the tenant model to pair output for this case.
        async def tenant_pairs(http, prompt, **kwargs):
            routing_provider.calls.append({"prompt": prompt, **kwargs})
            return PAIRS_JSON

        routing_provider.generate = tenant_pairs
        output = await env.run(activity.run, _input(teacher=None))
        assert output.pair_count == 1
        assert all(c["model"] == TENANT_MODEL for c in routing_provider.calls)

    @pytest.mark.asyncio
    async def test_malformed_teacher_is_non_retryable(self, routing_provider):
        activity = GeneratePairsActivity(
            _infra(_settings(), FakeS3({"chunks.jsonl": _chunks_jsonl()}), FakeDB(TENANT_SETTINGS))
        )
        env = ActivityEnvironment()
        with pytest.raises(ApplicationError) as exc_info:
            await env.run(activity.run, _input(teacher={"model": TEACHER_MODEL}))
        assert exc_info.value.non_retryable
        assert routing_provider.calls == []

    @pytest.mark.asyncio
    async def test_unsafe_teacher_url_is_non_retryable(self, routing_provider):
        settings = _settings(url_guard_enabled=True)
        activity = GeneratePairsActivity(
            _infra(settings, FakeS3({"chunks.jsonl": _chunks_jsonl()}), FakeDB(None))
        )
        env = ActivityEnvironment()
        with pytest.raises(ApplicationError) as exc_info:
            await env.run(
                activity.run,
                _input(teacher=_teacher_block(api_base_url="http://127.0.0.1:8000/v1")),
            )
        assert exc_info.value.non_retryable
        assert routing_provider.calls == []


class TestScopeRunKey:
    def test_teacher_changes_the_key(self):
        teacher = TeacherConfig(api_base_url="https://t.example/v1", model="m")
        other = TeacherConfig(api_base_url="https://t.example/v1", model="other")
        assert scope_run_key("run-1", None) == "run-1"
        assert scope_run_key("run-1", teacher) != "run-1"
        assert scope_run_key("run-1", teacher) != scope_run_key("run-1", other)

    def test_empty_key_stays_disabled(self):
        teacher = TeacherConfig(api_base_url="https://t.example/v1", model="m")
        assert scope_run_key("", teacher) == ""


class TestIncludeCot:
    @pytest.mark.asyncio
    async def test_cot_instruction_only_when_enabled(self):
        prompts: list[str] = []

        async def capture(prompt: str) -> str:
            prompts.append(prompt)
            return PAIRS_JSON

        for include_cot in (False, True):
            generator = LlmPairGenerator(llm_call=capture, include_cot=include_cot)
            await generator.generate(
                chunk_text="text " * 20,
                task_type="reasoning",
                guidance="",
                facet=None,
                count=1,
            )
        assert "step-by-step reasoning" not in prompts[0]
        assert "step-by-step reasoning" in prompts[1]


PAIRS_FILE = json.dumps(
    {
        "instruction": "What does the document say about chunk zero?",
        "response": "The document explains chunk zero in detail with grounded content.",
        "doc_id": "d0",
        "chunk_id": "c0",
    }
).encode("utf-8")


class TestBuildDatasetProvenance:
    async def _run(self, teacher: dict | None) -> tuple[FakeDB, object]:
        db = FakeDB()
        activity = BuildDatasetActivity(
            _infra(
                SimpleNamespace(dataset_filter_backend="heuristic", dedup_backend="hash"),
                FakeS3({"pairs.jsonl": PAIRS_FILE}),
                db,
            )
        )
        env = ActivityEnvironment()
        output = await env.run(
            activity.run,
            BuildDatasetInput(
                tenant_id="11111111-1111-1111-1111-111111111111",
                project_id="22222222-2222-2222-2222-222222222222",
                dataset_id="33333333-3333-3333-3333-333333333333",
                pairs_storage_path="pairs.jsonl",
                teacher=teacher,
            ),
        )
        return db, output

    @pytest.mark.asyncio
    async def test_provenance_written_without_credentials(self):
        db, output = await self._run(_teacher_block(api_key=None))
        assert output.pair_count == 1

        query, args = db.executed[-1]
        config_json = args[-1]
        config = json.loads(config_json)
        assert config["teacher"]["host"] == "teacher.example.com"
        assert config["teacher"]["model"] == TEACHER_MODEL
        assert config["teacher"]["policy"] == "allowed"
        assert config["teacher"]["cot"] is False
        assert "generated_at" in config["teacher"]
        assert "api_key" not in config_json

    @pytest.mark.asyncio
    async def test_existing_provenance_is_preserved_by_sql(self):
        db, _ = await self._run(_teacher_block(api_key=None))
        query, _ = db.executed[-1]
        assert "datasets.config ? 'teacher'" in query
        assert "THEN datasets.config" in query

    @pytest.mark.asyncio
    async def test_no_teacher_writes_empty_config(self):
        db, _ = await self._run(None)
        _, args = db.executed[-1]
        assert json.loads(args[-1]) == {}


class TestWireCompatibility:
    def test_new_fields_are_trailing_with_defaults(self):
        pair_fields = list(GenerateSyntheticPairsInput.__dataclass_fields__)
        assert pair_fields[-1] == "teacher"
        build_fields = list(BuildDatasetInput.__dataclass_fields__)
        assert build_fields[-1] == "teacher"
        assert GenerateSyntheticPairsInput.__dataclass_fields__["teacher"].default is None
        assert BuildDatasetInput.__dataclass_fields__["teacher"].default is None
