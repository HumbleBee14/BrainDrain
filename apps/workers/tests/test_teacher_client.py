"""TeacherClient guarantees: key handling, URL guard ordering, log hygiene."""

import logging
from types import SimpleNamespace

import pytest

from src.backends import llm_provider
from src.secret_cipher import encrypt_secret
from src.teacher import (
    TeacherClient,
    TeacherConfig,
    build_provenance,
    parse_teacher_config,
    read_provenance,
    teacher_host,
)
from src.url_guard import UnsafeUrlError

KEY_B64 = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="
PLAINTEXT_KEY = "sk-teacher-secret-123"


class _CapturingProvider:
    def __init__(self):
        self.calls: list[dict] = []

    async def generate(self, http, prompt, **kwargs):
        self.calls.append({"prompt": prompt, **kwargs})
        return "teacher says hi"


@pytest.fixture()
def capturing_provider():
    provider = _CapturingProvider()
    llm_provider.register("teacher_test_capture", lambda: provider)
    yield provider
    del llm_provider._REGISTRY["teacher_test_capture"]


def _settings(**overrides):
    values = {
        "llm_provider_backend": "teacher_test_capture",
        "llm_max_tokens": 2000,
        "settings_encryption_key": KEY_B64,
        "url_guard_enabled": False,
        "environment": "development",
    }
    values.update(overrides)
    return SimpleNamespace(**values)


def _config(**overrides):
    values = {
        "api_base_url": "https://teacher.example.com/v1",
        "model": "big-teacher-72b",
        "api_key": encrypt_secret(PLAINTEXT_KEY, KEY_B64),
    }
    values.update(overrides)
    return TeacherConfig(**values)


class TestParseTeacherConfig:
    def test_none_passes_through(self):
        assert parse_teacher_config(None) is None

    def test_non_dict_rejected(self):
        with pytest.raises(ValueError):
            parse_teacher_config("https://teacher.example.com")

    @pytest.mark.parametrize("missing", ["api_base_url", "model"])
    def test_missing_required_field_rejected(self, missing):
        raw = {"api_base_url": "https://t.example.com/v1", "model": "m"}
        raw[missing] = "  "
        with pytest.raises(ValueError):
            parse_teacher_config(raw)

    def test_defaults_and_trimming(self):
        cfg = parse_teacher_config(
            {"api_base_url": " https://t.example.com/v1 ", "model": " m ", "policy": ""}
        )
        assert cfg == TeacherConfig(
            api_base_url="https://t.example.com/v1",
            model="m",
            api_key="",
            policy="unknown",
            include_cot=False,
        )

    def test_full_block_parses(self):
        cfg = parse_teacher_config(
            {
                "api_base_url": "https://t.example.com/v1",
                "model": "m",
                "api_key": "enc:v1:abc",
                "policy": "restricted",
                "include_cot": True,
            }
        )
        assert cfg.api_key == "enc:v1:abc"
        assert cfg.policy == "restricted"
        assert cfg.include_cot is True


class TestTeacherClient:
    @pytest.mark.asyncio
    async def test_key_decrypted_in_memory_for_the_request(self, capturing_provider):
        client = TeacherClient(_config(), _settings())
        llm_call = client.make_llm_call(http=None, temperature=0.7)
        await llm_call("prompt")
        assert capturing_provider.calls[0]["api_key"] == PLAINTEXT_KEY
        assert capturing_provider.calls[0]["model"] == "big-teacher-72b"

    @pytest.mark.asyncio
    async def test_url_guard_blocks_before_any_request(self, capturing_provider):
        client = TeacherClient(
            _config(api_base_url="http://127.0.0.1:8000/v1"),
            _settings(url_guard_enabled=True),
        )
        llm_call = client.make_llm_call(http=None, temperature=0.7)
        with pytest.raises(UnsafeUrlError):
            await llm_call("prompt")
        assert capturing_provider.calls == []

    @pytest.mark.asyncio
    async def test_no_key_material_in_repr_or_logs(self, capturing_provider, caplog):
        config = _config()
        client = TeacherClient(config, _settings())
        with caplog.at_level(logging.DEBUG):
            await client.make_llm_call(http=None, temperature=0.7)("prompt")
        exposed = repr(client) + str(client) + caplog.text
        assert PLAINTEXT_KEY not in exposed
        assert config.api_key not in exposed

    def test_fingerprint_tracks_identity_not_key(self):
        base = _config()
        assert base.fingerprint() == _config(api_key="enc:v1:other").fingerprint()
        assert base.fingerprint() != _config(model="other-model").fingerprint()
        assert (
            base.fingerprint() != _config(api_base_url="https://elsewhere.example/v1").fingerprint()
        )


class TestProvenance:
    def test_build_and_read_roundtrip(self):
        block = build_provenance(_config(), generated_at="2026-08-04T00:00:00Z")
        assert block == {
            "host": "teacher.example.com",
            "model": "big-teacher-72b",
            "policy": "unknown",
            "cot": False,
            "generated_at": "2026-08-04T00:00:00Z",
        }
        assert read_provenance({"teacher": block}) == block

    def test_read_rejects_incomplete_blocks(self):
        assert read_provenance(None) is None
        assert read_provenance({}) is None
        assert read_provenance({"teacher": "big-teacher"}) is None
        assert read_provenance({"teacher": {"model": "m"}}) is None

    def test_host_falls_back_to_raw_value(self):
        assert teacher_host("not a url") == "not a url"
