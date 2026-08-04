"""The sole gateway to a teacher LLM endpoint.

Teacher credentials arrive in workflow payloads SecretCipher-encrypted
(enc:v1). This client decrypts them in memory immediately before each
request and never stores or logs the plaintext. The SSRF URL guard runs on
the teacher base URL before the first request — enforced by construction
here, not by call-site discipline.
"""

import asyncio
import hashlib
import logging
from dataclasses import dataclass

import httpx

from src.backends.llm_provider import get as get_llm_provider
from src.secret_cipher import decrypt_secret
from src.url_guard import assert_safe_public_url, url_guard_active

logger = logging.getLogger("platform.teacher")


@dataclass(frozen=True)
class TeacherConfig:
    """Teacher endpoint description as carried in workflow payloads.

    `api_key` is either empty or SecretCipher-encrypted; a plaintext key
    must never be placed here (payloads are durable Temporal history).
    """

    api_base_url: str
    model: str
    api_key: str = ""
    policy: str = "unknown"
    include_cot: bool = False

    def fingerprint(self) -> str:
        """Short stable id of the teacher identity (host + model, no key).

        Folded into checkpoint run keys so pairs checkpointed under one
        teacher are never resumed under another.
        """
        raw = f"{self.api_base_url}|{self.model}"
        return hashlib.sha256(raw.encode("utf-8")).hexdigest()[:8]


def parse_teacher_config(raw: dict | None) -> TeacherConfig | None:
    """Validate the wire-format teacher block into a typed config.

    Returns None for an absent block; raises ValueError (fail loud, not a
    silent fallback to the tenant LLM) for a present-but-malformed one.
    """
    if raw is None:
        return None
    if not isinstance(raw, dict):
        raise ValueError("teacher config must be a JSON object")

    api_base_url = raw.get("api_base_url")
    model = raw.get("model")
    if not isinstance(api_base_url, str) or not api_base_url.strip():
        raise ValueError("teacher config is missing api_base_url")
    if not isinstance(model, str) or not model.strip():
        raise ValueError("teacher config is missing model")

    api_key = raw.get("api_key")
    policy = raw.get("policy")
    return TeacherConfig(
        api_base_url=api_base_url.strip(),
        model=model.strip(),
        api_key=api_key.strip() if isinstance(api_key, str) else "",
        policy=policy.strip() if isinstance(policy, str) and policy.strip() else "unknown",
        include_cot=raw.get("include_cot") is True,
    )


class TeacherClient:
    """Produces `llm_call` closures bound to the teacher endpoint.

    The closures match `src.datagen.impls.LlmCall`, so a teacher slots into
    the datagen registry exactly like the tenant LLM does.

    The teacher deliberately does NOT share the tenant-LLM circuit breaker:
    a flaky teacher endpoint must not open the breaker for the tenant's own
    judge and facet calls (and vice versa).
    """

    def __init__(self, config: TeacherConfig, settings):
        self._config = config
        self._settings = settings
        self._provider = get_llm_provider(settings.llm_provider_backend)
        self._url_checked = False

    @property
    def model(self) -> str:
        return self._config.model

    @property
    def api_base_url(self) -> str:
        return self._config.api_base_url

    @property
    def config(self) -> TeacherConfig:
        return self._config

    def __repr__(self) -> str:
        return (
            f"TeacherClient(model={self._config.model!r}, "
            f"api_base_url={self._config.api_base_url!r})"
        )

    async def ensure_url_allowed(self) -> None:
        """Teacher URLs are user-supplied — always re-validated before use.

        Runs automatically before the first request; activities may also call
        it up front to fail fast before any other work.
        """
        if self._url_checked:
            return
        if url_guard_active(self._settings):
            await asyncio.to_thread(assert_safe_public_url, self._config.api_base_url)
        self._url_checked = True

    def _plaintext_key(self) -> str:
        """Decrypt at call time; the plaintext is never stored on the instance."""
        if not self._config.api_key:
            return ""
        return decrypt_secret(self._config.api_key, self._settings.settings_encryption_key)

    def make_llm_call(self, http: httpx.AsyncClient, temperature: float):
        async def llm_call(prompt: str) -> str:
            await self.ensure_url_allowed()
            return await self._provider.generate(
                http,
                prompt,
                model=self._config.model,
                api_base_url=self._config.api_base_url,
                api_key=self._plaintext_key(),
                max_tokens=self._settings.llm_max_tokens,
                temperature=temperature,
            )

        return llm_call
