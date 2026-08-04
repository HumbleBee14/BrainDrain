"""Byte-identity guard between a teacher and student tokenizer.

Offline logit/KL distillation trains a student against a teacher's
precomputed per-token top-k logprobs. The student only reads those targets
correctly if it tokenizes text exactly the way the teacher did — one
differing special token, added token, or chat-template character silently
shifts every training target and corrupts the run. This module is the sole
place that answers "are these two tokenizers byte-identical."

It compares four artifact buckets per model, each canonicalized before
hashing so a re-serialized-but-semantically-identical file never reads as a
mismatch:

- vocabulary/merges (`tokenizer.json`, or `vocab.json` + `merges.txt`)
- `special_tokens_map.json`
- `added_tokens.json`, or the embedded `added_tokens` section of
  `tokenizer.json` when there is no standalone file
- the chat template (`chat_template.jinja`, or the `chat_template` field of
  `tokenizer_config.json`)

Fetching is behind an injectable `ArtifactFetcher` so tests never touch the
network — they hand in a fetcher backed by on-disk fixtures. The default
fetcher uses `huggingface_hub.hf_hub_download`, imported lazily to keep this
module importable on workers that don't have the `ml` extra installed.
"""

import hashlib
import json
from collections.abc import Callable
from dataclasses import dataclass

ARTIFACT_VOCAB = "vocab"
ARTIFACT_SPECIAL_TOKENS_MAP = "special_tokens_map"
ARTIFACT_ADDED_TOKENS = "added_tokens"
ARTIFACT_CHAT_TEMPLATE = "chat_template"

_ARTIFACT_ORDER = (
    ARTIFACT_VOCAB,
    ARTIFACT_SPECIAL_TOKENS_MAP,
    ARTIFACT_ADDED_TOKENS,
    ARTIFACT_CHAT_TEMPLATE,
)

_TOKENIZER_JSON = "tokenizer.json"
_VOCAB_JSON = "vocab.json"
_MERGES_TXT = "merges.txt"
_SPECIAL_TOKENS_MAP_JSON = "special_tokens_map.json"
_ADDED_TOKENS_JSON = "added_tokens.json"
_CHAT_TEMPLATE_JINJA = "chat_template.jinja"
_TOKENIZER_CONFIG_JSON = "tokenizer_config.json"

_ABSENT = "ABSENT"


class TokenizerArtifactFetchError(Exception):
    """A tokenizer artifact could not be retrieved from the Hub.

    Distinct from "this optional file does not exist in the repo" (a normal
    outcome handled internally) — this means the fetch itself failed, so
    callers can retry rather than treat it as a tokenizer incompatibility.
    """


ArtifactFetcher = Callable[[str, str, str | None, str], bytes | None]
"""(model_id, filename, revision, hf_token) -> raw file bytes, or None if
the file does not exist in that model's repo."""


@dataclass(frozen=True)
class ArtifactHash:
    """A hashed tokenizer artifact. `sha256` is None when the artifact is absent
    from that model's repo — absence on both sides is still a match."""

    name: str
    sha256: str | None


@dataclass(frozen=True)
class ModelTokenizerHashes:
    model_id: str
    revision: str | None
    artifacts: tuple[ArtifactHash, ...]
    combined_hash: str

    def hash_for(self, artifact_name: str) -> str | None:
        for artifact in self.artifacts:
            if artifact.name == artifact_name:
                return artifact.sha256
        return None


@dataclass(frozen=True)
class TokenizerIdentityResult:
    teacher: ModelTokenizerHashes
    student: ModelTokenizerHashes
    compatible: bool
    mismatched_artifacts: tuple[str, ...]


_hash_cache: dict[tuple[str, str | None], ModelTokenizerHashes] = {}


def clear_cache() -> None:
    """Drop all cached per-model hashes (tests only need isolation, not prod)."""
    _hash_cache.clear()


def compute_tokenizer_hashes(
    model_id: str,
    *,
    revision: str | None = None,
    hf_token: str = "",
    fetcher: ArtifactFetcher | None = None,
) -> ModelTokenizerHashes:
    """Fetch, canonicalize, and hash one model's tokenizer artifacts.

    Cached in-process per (model_id, revision) so repeated calls (e.g. the
    preflight check followed by the extraction workflow's hard re-check)
    don't re-download.
    """
    cache_key = (model_id, revision)
    cached = _hash_cache.get(cache_key)
    if cached is not None:
        return cached

    active_fetcher = fetcher or _default_fetcher

    vocab_hash, tokenizer_json_raw = _vocab_bucket_hash(
        active_fetcher, model_id, revision, hf_token
    )
    artifacts = (
        ArtifactHash(ARTIFACT_VOCAB, vocab_hash),
        ArtifactHash(
            ARTIFACT_SPECIAL_TOKENS_MAP,
            _special_tokens_map_hash(active_fetcher, model_id, revision, hf_token),
        ),
        ArtifactHash(
            ARTIFACT_ADDED_TOKENS,
            _added_tokens_hash(active_fetcher, model_id, revision, hf_token, tokenizer_json_raw),
        ),
        ArtifactHash(
            ARTIFACT_CHAT_TEMPLATE,
            _chat_template_hash(active_fetcher, model_id, revision, hf_token),
        ),
    )

    result = ModelTokenizerHashes(
        model_id=model_id,
        revision=revision,
        artifacts=artifacts,
        combined_hash=_combined_hash(artifacts),
    )
    _hash_cache[cache_key] = result
    return result


def check_tokenizer_identity(
    teacher_model_id: str,
    student_model_id: str,
    *,
    teacher_revision: str | None = None,
    student_revision: str | None = None,
    hf_token: str = "",
    fetcher: ArtifactFetcher | None = None,
) -> TokenizerIdentityResult:
    """Compare a teacher and student tokenizer artifact-for-artifact.

    `compatible` is true only when every artifact bucket matches exactly
    (including both sides having the same artifact absent). Used both as the
    API-callable preflight behind the UX recommendation card and as the
    extraction workflow's hard, non-retryable re-check.
    """
    teacher = compute_tokenizer_hashes(
        teacher_model_id, revision=teacher_revision, hf_token=hf_token, fetcher=fetcher
    )
    student = compute_tokenizer_hashes(
        student_model_id, revision=student_revision, hf_token=hf_token, fetcher=fetcher
    )

    mismatched = tuple(
        name for name in _ARTIFACT_ORDER if teacher.hash_for(name) != student.hash_for(name)
    )

    return TokenizerIdentityResult(
        teacher=teacher,
        student=student,
        compatible=not mismatched,
        mismatched_artifacts=mismatched,
    )


def _combined_hash(artifacts: tuple[ArtifactHash, ...]) -> str:
    serialized = "\x01".join(f"{a.name}\x00{a.sha256 or _ABSENT}" for a in artifacts)
    return _sha256_hex(serialized.encode("utf-8"))


def _canonicalize_json(raw: bytes) -> bytes:
    data = json.loads(raw.decode("utf-8"))
    return json.dumps(data, sort_keys=True, separators=(",", ":")).encode("utf-8")


def _canonicalize_text(raw: bytes) -> bytes:
    text = raw.decode("utf-8").replace("\r\n", "\n").replace("\r", "\n")
    normalized = "\n".join(line.rstrip() for line in text.split("\n")).strip("\n")
    return normalized.encode("utf-8")


def _sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _vocab_bucket_hash(
    fetcher: ArtifactFetcher, model_id: str, revision: str | None, hf_token: str
) -> tuple[str | None, bytes | None]:
    """Hash the vocabulary bucket; also returns raw `tokenizer.json` bytes so
    the added-tokens bucket can reuse them without a second fetch."""
    tokenizer_json = fetcher(model_id, _TOKENIZER_JSON, revision, hf_token)
    if tokenizer_json is not None:
        canonical = b"tokenizer.json\x00" + _canonicalize_json(tokenizer_json)
        return _sha256_hex(canonical), tokenizer_json

    vocab_json = fetcher(model_id, _VOCAB_JSON, revision, hf_token)
    merges_txt = fetcher(model_id, _MERGES_TXT, revision, hf_token)
    if vocab_json is None and merges_txt is None:
        return None, None

    parts = []
    if vocab_json is not None:
        parts.append(b"vocab.json\x00" + _canonicalize_json(vocab_json))
    if merges_txt is not None:
        parts.append(b"merges.txt\x00" + _canonicalize_text(merges_txt))
    return _sha256_hex(b"\x01".join(parts)), None


def _special_tokens_map_hash(
    fetcher: ArtifactFetcher, model_id: str, revision: str | None, hf_token: str
) -> str | None:
    raw = fetcher(model_id, _SPECIAL_TOKENS_MAP_JSON, revision, hf_token)
    if raw is None:
        return None
    return _sha256_hex(_canonicalize_json(raw))


def _added_tokens_hash(
    fetcher: ArtifactFetcher,
    model_id: str,
    revision: str | None,
    hf_token: str,
    tokenizer_json_raw: bytes | None,
) -> str | None:
    standalone = fetcher(model_id, _ADDED_TOKENS_JSON, revision, hf_token)
    if standalone is not None:
        return _sha256_hex(_canonicalize_json(standalone))

    if tokenizer_json_raw is None:
        return None

    data = json.loads(tokenizer_json_raw.decode("utf-8"))
    if "added_tokens" not in data:
        return None
    section = json.dumps(data["added_tokens"], sort_keys=True, separators=(",", ":"))
    return _sha256_hex(section.encode("utf-8"))


def _chat_template_hash(
    fetcher: ArtifactFetcher, model_id: str, revision: str | None, hf_token: str
) -> str | None:
    jinja = fetcher(model_id, _CHAT_TEMPLATE_JINJA, revision, hf_token)
    if jinja is not None:
        return _sha256_hex(_canonicalize_text(jinja))

    tokenizer_config = fetcher(model_id, _TOKENIZER_CONFIG_JSON, revision, hf_token)
    if tokenizer_config is None:
        return None

    data = json.loads(tokenizer_config.decode("utf-8"))
    template = data.get("chat_template")
    if template is None:
        return None
    if isinstance(template, str):
        return _sha256_hex(_canonicalize_text(template.encode("utf-8")))
    canonical = json.dumps(template, sort_keys=True, separators=(",", ":"))
    return _sha256_hex(canonical.encode("utf-8"))


def _default_fetcher(
    model_id: str, filename: str, revision: str | None, hf_token: str
) -> bytes | None:
    from huggingface_hub import hf_hub_download
    from huggingface_hub.errors import (
        EntryNotFoundError,
        GatedRepoError,
        HfHubHTTPError,
        RepositoryNotFoundError,
        RevisionNotFoundError,
    )

    try:
        path = hf_hub_download(
            repo_id=model_id,
            filename=filename,
            revision=revision,
            token=hf_token or None,
        )
    except EntryNotFoundError:
        return None
    except (
        RepositoryNotFoundError,
        RevisionNotFoundError,
        GatedRepoError,
        HfHubHTTPError,
        OSError,
    ) as exc:
        raise TokenizerArtifactFetchError(
            f"Failed to fetch {filename!r} for {model_id!r}: {exc}"
        ) from exc

    with open(path, "rb") as fh:
        return fh.read()
