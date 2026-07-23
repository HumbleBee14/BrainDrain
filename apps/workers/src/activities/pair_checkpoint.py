"""Per-chunk checkpointing for synthetic pair generation.

Pair generation makes one (often several) LLM call per document chunk. On a
long run, a transient failure near the end would otherwise discard every
already-generated chunk and force Temporal to regenerate from scratch. A
checkpoint persists each chunk's *finished* pairs (post-faithfulness) to S3 as
it completes; on retry, completed chunks are loaded and skipped.

Correctness relies on two properties:
- The checkpoint prefix is stable across retries of a single activity execution
  (derived from the Temporal workflow-run + activity id) but unique across
  executions, so a fresh generation never inherits a stale checkpoint.
- Each chunk is one object (`<index>.json`); writes never rewrite prior chunks,
  so persistence is O(1) per chunk rather than O(n) re-serialisation.
"""

import json
from typing import Protocol, runtime_checkable


@runtime_checkable
class Checkpoint(Protocol):
    """Resumable store of finished pairs keyed by chunk index."""

    def load(self) -> dict[int, list[dict]]:
        """Return {chunk_index: finished pair records} persisted so far."""
        ...

    def save(self, chunk_index: int, pairs: list[dict]) -> None:
        """Durably persist one chunk's finished pairs (may be empty)."""
        ...

    def clear(self) -> None:
        """Remove all checkpoint objects (best-effort cleanup on success)."""
        ...


class NullCheckpoint:
    """No-op checkpoint — generation runs all-or-nothing, no extra writes."""

    def load(self) -> dict[int, list[dict]]:
        return {}

    def save(self, chunk_index: int, pairs: list[dict]) -> None:
        return None

    def clear(self) -> None:
        return None


class PairCheckpoint:
    """S3-backed per-chunk checkpoint.

    One object per chunk under ``prefix`` (``<prefix><index>.json``). ``s3`` is a
    synchronous boto3 client, matching how the generation activity already calls
    S3.
    """

    def __init__(self, s3, bucket: str, prefix: str):
        self._s3 = s3
        self._bucket = bucket
        self._prefix = prefix

    def _key(self, chunk_index: int) -> str:
        return f"{self._prefix}{chunk_index}.json"

    def load(self) -> dict[int, list[dict]]:
        completed: dict[int, list[dict]] = {}
        continuation: str | None = None
        while True:
            kwargs = {"Bucket": self._bucket, "Prefix": self._prefix}
            if continuation:
                kwargs["ContinuationToken"] = continuation
            resp = self._s3.list_objects_v2(**kwargs)
            for obj in resp.get("Contents", []):
                body = self._s3.get_object(Bucket=self._bucket, Key=obj["Key"])
                record = json.loads(body["Body"].read().decode("utf-8"))
                completed[int(record["chunk_index"])] = record["pairs"]
            if not resp.get("IsTruncated"):
                break
            continuation = resp.get("NextContinuationToken")
        return completed

    def save(self, chunk_index: int, pairs: list[dict]) -> None:
        payload = json.dumps({"chunk_index": chunk_index, "pairs": pairs}, ensure_ascii=False)
        self._s3.put_object(
            Bucket=self._bucket,
            Key=self._key(chunk_index),
            Body=payload.encode("utf-8"),
            ContentType="application/json",
        )

    def clear(self) -> None:
        continuation: str | None = None
        while True:
            kwargs = {"Bucket": self._bucket, "Prefix": self._prefix}
            if continuation:
                kwargs["ContinuationToken"] = continuation
            resp = self._s3.list_objects_v2(**kwargs)
            keys = [{"Key": obj["Key"]} for obj in resp.get("Contents", [])]
            if keys:
                self._s3.delete_objects(Bucket=self._bucket, Delete={"Objects": keys})
            if not resp.get("IsTruncated"):
                break
            continuation = resp.get("NextContinuationToken")
