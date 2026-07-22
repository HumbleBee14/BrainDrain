"""Unit tests for the S3 LoRA resolver.

vLLM and boto3 are stubbed so the test runs without GPU/ML deps installed:
    python -m pytest infra/serving/vllm_s3_lora_resolver/tests
"""

from __future__ import annotations

import asyncio
import json
import sys
import types
from pathlib import Path


def _install_stubs() -> None:
    """Inject fake vllm + boto3 modules before importing the resolver."""
    if "vllm.lora.request" not in sys.modules:
        req_mod = types.ModuleType("vllm.lora.request")

        class LoRARequest:  # noqa: D401 - stub
            def __init__(self, lora_name, lora_int_id, lora_path):
                self.lora_name = lora_name
                self.lora_int_id = lora_int_id
                self.lora_path = lora_path

        req_mod.LoRARequest = LoRARequest

        res_mod = types.ModuleType("vllm.lora.resolver")

        class LoRAResolver:  # noqa: D401 - stub base
            pass

        class _Registry:
            registered: dict = {}

            @classmethod
            def register_resolver(cls, name, resolver):
                cls.registered[name] = resolver

        res_mod.LoRAResolver = LoRAResolver
        res_mod.LoRAResolverRegistry = _Registry

        vllm = types.ModuleType("vllm")
        lora = types.ModuleType("vllm.lora")
        sys.modules["vllm"] = vllm
        sys.modules["vllm.lora"] = lora
        sys.modules["vllm.lora.request"] = req_mod
        sys.modules["vllm.lora.resolver"] = res_mod


_install_stubs()

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import s3_lora_resolver as mod  # noqa: E402


class _FakeS3:
    """Minimal boto3 S3 client double backed by a local source directory."""

    def __init__(self, source: Path, objects: list[str]):
        self._source = source
        self._objects = objects

    def get_paginator(self, _name):
        objects = self._objects

        class _Paginator:
            def paginate(self, Bucket, Prefix):  # noqa: N803 - boto3 kwarg names
                contents = [{"Key": k} for k in objects if k.startswith(Prefix)]
                yield {"Contents": contents}

        return _Paginator()

    def download_file(self, _bucket, key, dest):
        (self._source / key).parent.mkdir(parents=True, exist_ok=True)
        data = (self._source / key).read_bytes()
        Path(dest).write_bytes(data)


def _seed_adapter(root: Path, prefix: str, base_model: str) -> list[str]:
    keys = [f"{prefix}/adapter_config.json", f"{prefix}/adapter_model.safetensors"]
    (root / f"{prefix}").mkdir(parents=True, exist_ok=True)
    (root / keys[0]).write_text(json.dumps({"base_model_name_or_path": base_model}))
    (root / keys[1]).write_bytes(b"weights")
    return keys


def _make_resolver(tmp_path, monkeypatch, source, objects, **kw):
    resolver = mod.S3LoRAResolver(bucket="test-bucket", cache_dir=str(tmp_path / "cache"), **kw)
    monkeypatch.setattr(resolver, "_client", lambda: _FakeS3(source, objects))
    return resolver


def test_resolves_and_downloads(tmp_path, monkeypatch):
    src = tmp_path / "src"
    keys = _seed_adapter(src, "adapters/t1/m1", "unsloth/Qwen2.5-0.5B-Instruct")
    resolver = _make_resolver(tmp_path, monkeypatch, src, keys)

    req = asyncio.run(resolver.resolve_lora("unsloth/Qwen2.5-0.5B-Instruct", "adapters/t1/m1"))
    assert req is not None
    assert req.lora_name == "adapters/t1/m1"
    assert (Path(req.lora_path) / "adapter_config.json").is_file()
    assert (Path(req.lora_path) / "adapter_model.safetensors").is_file()


def test_missing_config_returns_none(tmp_path, monkeypatch):
    src = tmp_path / "src"
    (src / "adapters/bad").mkdir(parents=True)
    (src / "adapters/bad/readme.txt").write_text("no config here")
    resolver = _make_resolver(tmp_path, monkeypatch, src, ["adapters/bad/readme.txt"])
    assert asyncio.run(resolver.resolve_lora("m", "adapters/bad")) is None


def test_base_model_basename_match(tmp_path, monkeypatch):
    src = tmp_path / "src"
    keys = _seed_adapter(src, "adapters/t/m", "org/Qwen2.5-0.5B-Instruct")
    resolver = _make_resolver(tmp_path, monkeypatch, src, keys)
    # Served under a different org prefix but same model basename -> accepted.
    req = asyncio.run(resolver.resolve_lora("other/Qwen2.5-0.5B-Instruct", "adapters/t/m"))
    assert req is not None


def test_strict_base_model_rejects_mismatch(tmp_path, monkeypatch):
    src = tmp_path / "src"
    keys = _seed_adapter(src, "adapters/t/m", "meta/Llama-3-8B")
    resolver = _make_resolver(tmp_path, monkeypatch, src, keys, strict_base_model=True)
    assert asyncio.run(resolver.resolve_lora("unsloth/Qwen2.5-0.5B", "adapters/t/m")) is None


def test_cache_hit_skips_download(tmp_path, monkeypatch):
    src = tmp_path / "src"
    keys = _seed_adapter(src, "adapters/t/m", "unsloth/Qwen2.5-0.5B-Instruct")
    resolver = _make_resolver(tmp_path, monkeypatch, src, keys)
    asyncio.run(resolver.resolve_lora("unsloth/Qwen2.5-0.5B-Instruct", "adapters/t/m"))

    # Second call with a client that would raise if used proves the cache hit.
    def _boom():
        raise AssertionError("should not hit S3 on cache hit")

    monkeypatch.setattr(resolver, "_client", _boom)
    req = asyncio.run(resolver.resolve_lora("unsloth/Qwen2.5-0.5B-Instruct", "adapters/t/m"))
    assert req is not None
