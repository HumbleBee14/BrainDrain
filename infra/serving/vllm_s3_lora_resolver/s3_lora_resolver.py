"""S3-backed LoRA resolver plugin for vLLM.

vLLM resolves a LoRA adapter lazily the first time an unknown adapter name
appears in the ``model`` field of an inference request. This plugin fetches
that adapter from an S3-compatible object store (AWS S3, MinIO, Cloudflare R2),
caches it on local disk, and hands vLLM a ``LoRARequest`` pointing at the
cached directory.

The control plane addresses an adapter by its S3 key prefix (the same
``adapter_path`` stored on the model row, e.g. ``adapters/<tenant>/<job_id>/``).
That prefix is the ``lora_name`` vLLM passes here.

Enable it by launching vLLM with::

    VLLM_ALLOW_RUNTIME_LORA_UPDATING=true
    VLLM_PLUGINS=s3_lora_resolver
    VLLM_LORA_RESOLVER_CACHE_DIR=/var/lora-cache

and the S3 connection env vars documented in ``infra/serving/README.md``.
"""

from __future__ import annotations

import asyncio
import json
import logging
import os
from pathlib import Path

import boto3
from botocore.config import Config
from vllm.lora.request import LoRARequest
from vllm.lora.resolver import LoRAResolver, LoRAResolverRegistry

logger = logging.getLogger(__name__)

# Objects that make up a PEFT LoRA adapter directory. We download every object
# under the prefix, but require at least the config to consider it valid.
_ADAPTER_CONFIG = "adapter_config.json"


def _env(name: str, default: str | None = None) -> str | None:
    value = os.environ.get(name)
    return value if value not in (None, "") else default


class S3LoRAResolver(LoRAResolver):
    """Download LoRA adapters from an S3-compatible bucket on demand."""

    def __init__(
        self,
        bucket: str,
        cache_dir: str,
        *,
        endpoint_url: str | None = None,
        region: str | None = None,
        strict_base_model: bool = False,
    ) -> None:
        self._bucket = bucket
        self._cache_dir = Path(cache_dir)
        self._cache_dir.mkdir(parents=True, exist_ok=True)
        self._endpoint_url = endpoint_url
        self._region = region
        self._strict_base_model = strict_base_model
        # Guard so two concurrent requests for the same adapter don't race on
        # the same cache directory.
        self._locks: dict[str, asyncio.Lock] = {}

    def _client(self):
        # boto3 clients are cheap to create and not thread-safe to share, so we
        # build one per download call (which runs in a worker thread).
        return boto3.client(
            "s3",
            endpoint_url=self._endpoint_url,
            region_name=self._region,
            config=Config(retries={"max_attempts": 3, "mode": "standard"}),
        )

    async def resolve_lora(self, base_model_name: str, lora_name: str) -> LoRARequest | None:
        lock = self._locks.setdefault(lora_name, asyncio.Lock())
        async with lock:
            try:
                local_dir = await asyncio.to_thread(self._download, lora_name)
            except Exception:  # noqa: BLE001 — a resolver must never raise; return None
                logger.exception("S3 LoRA resolve failed for %r", lora_name)
                return None

        if local_dir is None:
            return None

        if not self._base_model_matches(base_model_name, local_dir):
            logger.warning(
                "Adapter %r base model mismatch (served=%r); rejecting",
                lora_name,
                base_model_name,
            )
            return None

        return LoRARequest(
            lora_name=lora_name,
            lora_int_id=abs(hash(lora_name)) % (2**31),
            lora_path=str(local_dir),
        )

    def _download(self, lora_name: str) -> Path | None:
        """Sync download of every object under the adapter prefix. Returns the
        local directory, or None if no adapter config was found."""
        prefix = lora_name.strip("/")
        target = self._cache_dir / prefix.replace("/", "__")

        # Already cached — trust it (adapters are immutable once trained).
        if (target / _ADAPTER_CONFIG).is_file():
            logger.info("Adapter %r already cached at %s", lora_name, target)
            return target

        client = self._client()
        paginator = client.get_paginator("list_objects_v2")
        downloaded = 0
        for page in paginator.paginate(Bucket=self._bucket, Prefix=f"{prefix}/"):
            for obj in page.get("Contents", []):
                key = obj["Key"]
                rel = key[len(prefix) + 1 :]
                if not rel:  # the prefix "directory" placeholder itself
                    continue
                dest = target / rel
                dest.parent.mkdir(parents=True, exist_ok=True)
                client.download_file(self._bucket, key, str(dest))
                downloaded += 1

        if not (target / _ADAPTER_CONFIG).is_file():
            logger.error(
                "No %s under s3://%s/%s (downloaded %d objects)",
                _ADAPTER_CONFIG,
                self._bucket,
                prefix,
                downloaded,
            )
            return None

        logger.info("Fetched adapter %r (%d objects) to %s", lora_name, downloaded, target)
        return target

    def _base_model_matches(self, served_model: str, adapter_dir: Path) -> bool:
        """Compare the served base model against the adapter's declared base.

        Accepts an exact match or a matching final path segment (handles
        ``org/Model`` vs a local path to the same model). When the adapter
        omits the field, accept it and let vLLM's loader do the final check.
        """
        try:
            config = json.loads((adapter_dir / _ADAPTER_CONFIG).read_text())
        except (OSError, ValueError):
            return not self._strict_base_model
        declared = config.get("base_model_name_or_path")
        if not declared:
            return True
        if declared == served_model:
            return True
        if declared.rstrip("/").split("/")[-1] == served_model.rstrip("/").split("/")[-1]:
            return True
        return not self._strict_base_model


def register_s3_resolver() -> None:
    """Entry point invoked by vLLM's plugin loader at startup.

    No-op (with a warning) if the bucket or cache dir env vars are missing, so
    the server still boots for non-LoRA use.
    """
    bucket = _env("S3_LORA_BUCKET") or _env("S3_BUCKET")
    cache_dir = _env("VLLM_LORA_RESOLVER_CACHE_DIR")
    if not bucket or not cache_dir:
        logger.warning(
            "S3 LoRA resolver not registered: set S3_LORA_BUCKET and "
            "VLLM_LORA_RESOLVER_CACHE_DIR to enable it"
        )
        return

    resolver = S3LoRAResolver(
        bucket=bucket,
        cache_dir=cache_dir,
        endpoint_url=_env("S3_ENDPOINT_URL") or _env("S3_ENDPOINT"),
        region=_env("S3_REGION") or _env("AWS_REGION"),
        strict_base_model=(_env("S3_LORA_STRICT_BASE_MODEL", "false") or "").lower() == "true",
    )
    LoRAResolverRegistry.register_resolver("S3 Resolver", resolver)
    logger.info(
        "Registered S3 LoRA resolver (bucket=%s, cache=%s, endpoint=%s)",
        bucket,
        cache_dir,
        resolver._endpoint_url or "default",
    )
