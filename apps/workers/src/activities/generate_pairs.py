"""Synthetic pair generation activity — creates instruction/response pairs from chunks.

Uses a configurable LLM API (OpenAI-compatible format) to generate
training data from document chunks. LLM provider config is resolved
per-tenant from the database at execution time. Structured generation
(PairGenerator), facet-diversity distribution, and a faithfulness
quality-gate (FaithfulnessScorer) are provided via src.datagen.registry.
"""

import hashlib
import json
import logging
import re
import uuid
from dataclasses import dataclass, field

import httpx
from temporalio import activity

from src import s3_paths
from src.activities.pair_checkpoint import Checkpoint, NullCheckpoint, PairCheckpoint
from src.backends.llm_provider import get as get_llm_provider
from src.circuit_breaker import CircuitBreakerOpen
from src.datagen.protocols import Facet, FaithfulnessScorer, GeneratedPair, PairGenerator
from src.datagen.registry import get_facet_expander, get_faithfulness_scorer, get_pair_generator
from src.failure_message import NO_LLM_KEY
from src.infra import InfraContainer
from src.tenant_config import get_tenant_llm_config

logger = logging.getLogger("platform.generate")

# Transport-level/transient errors that are tolerated per-chunk: one flaky
# call must not discard already-succeeded chunks. TimeoutError also catches
# asyncio.TimeoutError (an alias of it since Python 3.11). ValueError is
# deliberately excluded — that's a structured-parse/contract failure from
# the pair generator and must propagate (fail loud), not be swallowed here.
TRANSIENT_GENERATION_ERRORS = (httpx.HTTPError, TimeoutError, CircuitBreakerOpen)

# pairs_per_chunk multiplies into one LLM call per chunk; bound it so a stray
# large value can't fan out into a runaway number of generations.
MIN_PAIRS_PER_CHUNK = 1
MAX_PAIRS_PER_CHUNK = 50

# Golden holdout: a slice of chunks reserved for the document-grounded eval set.
# Below the chunk floor no holdout is taken — tiny documents need every chunk
# for training signal. The ratio is capped so a stray config value can never
# starve training of the majority of its source material.
MIN_CHUNKS_FOR_HOLDOUT = 10
MAX_HOLDOUT_RATIO = 0.25
# The golden set measures knowledge, not volume — a few well-grounded questions
# per held-out chunk suffice, independent of the training fan-out setting.
GOLDEN_PAIRS_PER_CHUNK = 3

# Facet-subtopic expansion: bound the fan-out so a stray config value cannot
# multiply the angle list (and prompt size) unreasonably.
MAX_SUBTOPICS_PER_FACET = 6
# How much sampled document text grounds each facet's subtopic expansion.
SUBTOPIC_DOC_SAMPLE_CHARS = 4000


def clamp_pairs_per_chunk(value: int) -> int:
    """Bound pairs_per_chunk to a sane range to cap generation fan-out."""
    return max(MIN_PAIRS_PER_CHUNK, min(value, MAX_PAIRS_PER_CHUNK))


def _chunk_fingerprint(index: int, chunk: dict) -> str:
    """Stable identity for holdout selection: ids when present, else content."""
    doc_id = chunk.get("doc_id") or ""
    chunk_id = chunk.get("chunk_id") or ""
    if doc_id or chunk_id:
        raw = f"{doc_id}:{chunk_id}"
    else:
        raw = f"{index}:{chunk.get('text', '')[:200]}"
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def facets_to_angles(facets: list[Facet], subtopics_by_facet: dict[str, list[str]]) -> list[Facet]:
    """Flatten facets and their subtopics into the rotation list ("angles").

    Every facet contributes itself first (so a facet with no subtopics — or a
    failed expansion — still steers generation exactly as before), followed by
    one angle per subtopic labeled "facet — subtopic". Angle ids extend the
    parent facet id, so provenance stays traceable. Rotating chunks across this
    longer list spreads samples over facet×subtopic cells instead of clustering
    on each facet's most obvious phrasing.
    """
    angles: list[Facet] = []
    for facet in facets:
        angles.append(facet)
        for n, sub in enumerate(subtopics_by_facet.get(facet.id, [])):
            angles.append(
                Facet(
                    id=f"{facet.id}.{n}",
                    label=f"{facet.label} — {sub}",
                    source_doc_id=facet.source_doc_id,
                    keep=True,
                )
            )
    return angles


def doc_sample_for_expansion(chunks: list[dict], limit: int = SUBTOPIC_DOC_SAMPLE_CHARS) -> str:
    """Sample text spread across the document to ground subtopic expansion.

    Takes evenly spaced chunks (not just the first ones) so the excerpt
    reflects the whole document's coverage, up to `limit` characters.
    """
    if not chunks:
        return ""
    parts: list[str] = []
    total = 0
    step = max(1, len(chunks) // 8)
    for chunk in chunks[::step]:
        text = chunk.get("text", "").strip()
        if not text:
            continue
        take = text[: max(0, limit - total)]
        if not take:
            break
        parts.append(take)
        total += len(take)
    return "\n\n".join(parts)


def select_holdout_chunks(chunks: list[dict], ratio: float) -> tuple[list[dict], list[dict]]:
    """Partition chunks into ``(training_chunks, holdout_chunks)`` deterministically.

    The holdout feeds the golden eval set: questions generated from chunks the
    model NEVER trains on, so evaluation measures document knowledge rather than
    memorization of training pairs. Selection is content-addressed (chunks sorted
    by fingerprint hash, top-N held out) — no RNG state — so retries, checkpoint
    resumes, and re-runs over the same chunk file always agree on the partition.

    No holdout is taken when the ratio is non-positive or there are fewer than
    ``MIN_CHUNKS_FOR_HOLDOUT`` chunks; the ratio is capped at
    ``MAX_HOLDOUT_RATIO``.
    """
    if ratio <= 0 or len(chunks) < MIN_CHUNKS_FOR_HOLDOUT:
        return chunks, []

    ratio = min(ratio, MAX_HOLDOUT_RATIO)
    holdout_count = max(1, round(len(chunks) * ratio))

    ranked = sorted(range(len(chunks)), key=lambda i: _chunk_fingerprint(i, chunks[i]))
    holdout_indices = set(ranked[:holdout_count])

    training = [c for i, c in enumerate(chunks) if i not in holdout_indices]
    holdout = [c for i, c in enumerate(chunks) if i in holdout_indices]
    return training, holdout


@dataclass
class GenerateSyntheticPairsInput:
    tenant_id: str
    project_id: str
    chunks_storage_path: str
    task_type: str
    pairs_per_chunk: int = 5
    guidance: str = ""
    facets: list[dict] | None = None
    # Fraction of chunks reserved for the golden (document-grounded) eval set.
    # 0 disables the holdout entirely.
    golden_holdout_ratio: float = 0.1
    # Subtopics to expand each facet into for generation diversity (rotating
    # chunks across facet×subtopic angles instead of the flat facet list).
    # 0 disables expansion.
    facet_subtopics: int = 3
    # Human-rated preview samples ({prompt, response, looks_good}) used as
    # few-shot calibration for the faithfulness judge. Defaulted so payloads
    # queued before this field existed still deserialize.
    rated: list[dict] = field(default_factory=list)


@dataclass
class GenerateSyntheticPairsOutput:
    pair_count: int
    storage_path: str
    golden_pair_count: int = 0
    golden_storage_path: str = ""


async def apply_faithfulness_gate(
    pairs: list[GeneratedPair],
    scorer: FaithfulnessScorer | None,
    *,
    enabled: bool,
) -> tuple[list[GeneratedPair], int]:
    """Drop pairs whose response isn't grounded in their source text.

    If the gate is disabled or no scorer is configured, all pairs pass through
    unchanged. Scorer failures propagate — a broken judge must not silently
    let unfaithful pairs through.
    """
    if not enabled or scorer is None:
        return pairs, 0

    kept: list[GeneratedPair] = []
    dropped = 0
    for pair in pairs:
        verdict = await scorer.score(pair=pair, source_text=pair.source_text)
        if verdict.consistent is True:
            kept.append(pair)
        else:
            dropped += 1
    return kept, dropped


def _finished_record(pair: GeneratedPair, chunk: dict, task_type: str) -> dict:
    """Serializable pair record carrying its source chunk's provenance.

    doc/chunk ids are attached at generation time (not via a later id()-keyed
    lookup) so a record survives being checkpointed to S3 and reloaded on retry.
    """
    return {
        "id": str(uuid.uuid4()),
        "doc_id": chunk.get("doc_id"),
        "chunk_id": chunk.get("chunk_id"),
        "task_type": task_type,
        "instruction": pair.prompt,
        "response": pair.response,
        "source_text": pair.source_text[:500],
    }


async def generate_pairs_with_checkpoint(
    chunks: list[dict],
    facets: list[Facet],
    pair_generator: PairGenerator,
    scorer: FaithfulnessScorer | None,
    checkpoint: Checkpoint,
    *,
    task_type: str,
    guidance: str,
    pairs_per_chunk: int,
    faithfulness_enabled: bool,
) -> tuple[list[dict], int, int]:
    """Generate finished pair records chunk-by-chunk, resuming from `checkpoint`.

    Each chunk is generated, faithfulness-gated, and its surviving records are
    persisted to the checkpoint before the next chunk starts. On retry, chunks
    already in the checkpoint are loaded and skipped, so a mid-run failure does
    not discard completed work. Faithfulness scoring is per-pair against its own
    source text, so applying the gate per chunk is equivalent to a single batch
    pass over all pairs.

    A single chunk hitting a timeout, rate limit, or transport error is tolerated
    (`TRANSIENT_GENERATION_ERRORS`) and left un-checkpointed so a retry
    regenerates it; a `ValueError` from the generator (malformed/missing LLM
    output) indicates a real bug and always propagates.

    Returns `(finished_records, dropped_unfaithful, failed_chunks)`. Raises if
    every attempted chunk failed transiently and nothing — checkpointed or new —
    was produced.
    """
    completed = checkpoint.load()
    records: list[dict] = []
    avoid: list[str] = []
    for i in sorted(completed):
        recs = completed[i]
        records.extend(recs)
        avoid.extend(r["instruction"] for r in recs)

    dropped_unfaithful = 0
    failed_chunks = 0

    for i, chunk in enumerate(chunks):
        if i in completed:
            continue
        activity.heartbeat()

        chunk_text = chunk.get("text", "")
        if len(chunk_text) < 50:
            # Nothing to generate, but mark the chunk done so a retry skips it.
            checkpoint.save(i, [])
            continue

        facet = facets[i % len(facets)] if facets else None
        try:
            pairs = await pair_generator.generate(
                chunk_text=chunk_text[:3000],
                task_type=task_type,
                guidance=guidance,
                facet=facet,
                count=pairs_per_chunk,
                avoid=list(avoid),
            )
        except TRANSIENT_GENERATION_ERRORS as exc:
            failed_chunks += 1
            activity.logger.warning("Chunk %d generation failed transiently, skipping: %s", i, exc)
            continue

        kept, dropped = await apply_faithfulness_gate(pairs, scorer, enabled=faithfulness_enabled)
        dropped_unfaithful += dropped

        recs = [_finished_record(pair, chunk, task_type) for pair in kept]
        checkpoint.save(i, recs)
        records.extend(recs)
        avoid.extend(r["instruction"] for r in recs)

    if failed_chunks > 0 and not records:
        raise RuntimeError(
            f"All {failed_chunks} chunk(s) failed synthetic pair generation due to "
            "transient errors; refusing to return an empty dataset."
        )

    return records, dropped_unfaithful, failed_chunks


class GeneratePairsActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="generate_synthetic_pairs")
    async def run(self, input: GenerateSyntheticPairsInput) -> GenerateSyntheticPairsOutput:
        """Generate instruction/response pairs from chunked text using LLM API."""
        s3 = self.infra.s3
        bucket = self.infra.s3_bucket
        settings = self.infra.settings

        # Resolve LLM config for this tenant (DB lookup, falls back to env var defaults)
        llm_config = await get_tenant_llm_config(
            db=self.infra.db,
            tenant_id=input.tenant_id,
            default_api_base_url=settings.llm_api_base_url,
            default_api_key=settings.llm_api_key,
            default_model=settings.llm_model,
            default_max_tokens=settings.llm_max_tokens,
            encryption_key=settings.settings_encryption_key,
            settings=settings,
        )

        if not llm_config.api_key:
            raise ValueError(NO_LLM_KEY)

        if llm_config.is_custom:
            logger.info(
                "Using tenant-specific LLM config for %s (provider: %s, model: %s)",
                input.tenant_id,
                llm_config.api_base_url,
                llm_config.model,
            )

        # Download chunks
        if not input.chunks_storage_path:
            return GenerateSyntheticPairsOutput(pair_count=0, storage_path="")

        response = s3.get_object(Bucket=bucket, Key=input.chunks_storage_path)
        chunks_data = response["Body"].read().decode("utf-8")
        chunks = [json.loads(line) for line in chunks_data.strip().split("\n") if line.strip()]

        if not chunks:
            return GenerateSyntheticPairsOutput(pair_count=0, storage_path="")

        provider = get_llm_provider(settings.llm_provider_backend)

        # Only user-kept facets steer generation; a discarded facet must not
        # leak back in via a stale `input.facets` payload. Missing `keep`
        # defaults to kept (matches the DTO default on the Rust side).
        facets = [Facet(**f) for f in input.facets if f.get("keep", True)] if input.facets else []

        # Reserve a deterministic slice of chunks for the golden eval set —
        # questions the model will be evaluated on but never trained on.
        training_chunks, holdout_chunks = select_holdout_chunks(chunks, input.golden_holdout_ratio)

        run_key = self._run_key()
        checkpoint = self._build_checkpoint(input, run_key)

        async with httpx.AsyncClient(timeout=120.0) as http:

            def make_llm_call(temperature: float):
                async def llm_call(prompt: str) -> str:
                    return await self.infra.circuit_breaker.call(
                        provider.generate,
                        http,
                        prompt,
                        model=llm_config.model,
                        api_base_url=llm_config.api_base_url,
                        api_key=llm_config.api_key,
                        max_tokens=llm_config.max_tokens,
                        temperature=temperature,
                    )

                return llm_call

            # Generation samples creatively; the faithfulness judge is scored
            # near-deterministically so verdicts are stable for the same inputs.
            pair_generator: PairGenerator = get_pair_generator(
                settings, make_llm_call(settings.generation_temperature)
            )
            scorer = get_faithfulness_scorer(
                settings, make_llm_call(settings.judge_temperature), calibration=input.rated
            )

            # Expand facets into facet×subtopic angles for generation diversity.
            # One LLM call per facet — negligible next to the per-chunk calls.
            # Best-effort by design: expansion only ADDS rotation angles, so any
            # failure (transient or malformed output) falls back to the base
            # facet and generation proceeds exactly as without expansion — a
            # diversity enhancer must never fail a dataset. Skipped when the
            # chunk count doesn't outnumber the facets (rotation never repeats
            # a facet in that case, so expansion buys nothing).
            angles = facets
            if facets and input.facet_subtopics > 0 and len(training_chunks) > len(facets):
                expander = get_facet_expander(
                    settings, make_llm_call(settings.generation_temperature)
                )
                num_subtopics = min(input.facet_subtopics, MAX_SUBTOPICS_PER_FACET)
                doc_sample = doc_sample_for_expansion(training_chunks)
                subtopics_by_facet: dict[str, list[str]] = {}
                for facet in facets:
                    activity.heartbeat()
                    try:
                        subtopics_by_facet[facet.id] = await expander.expand(
                            facet=facet,
                            doc_sample=doc_sample,
                            task_type=input.task_type,
                            guidance=input.guidance,
                            num_subtopics=num_subtopics,
                        )
                    except (*TRANSIENT_GENERATION_ERRORS, ValueError) as exc:
                        activity.logger.warning(
                            "Subtopic expansion failed for facet %r; using it unexpanded: %s",
                            facet.label,
                            exc,
                        )
                angles = facets_to_angles(facets, subtopics_by_facet)
                activity.logger.info(
                    "Facet expansion: %d facets -> %d angles", len(facets), len(angles)
                )

            pairs_per_chunk = clamp_pairs_per_chunk(input.pairs_per_chunk)
            records, dropped_unfaithful, failed_chunks = await generate_pairs_with_checkpoint(
                training_chunks,
                angles,
                pair_generator,
                scorer,
                checkpoint,
                task_type=input.task_type,
                guidance=input.guidance,
                pairs_per_chunk=pairs_per_chunk,
                faithfulness_enabled=settings.faithfulness_gate_enabled,
            )

            golden_records: list[dict] = []
            golden_checkpoint: Checkpoint = NullCheckpoint()
            if holdout_chunks and records:
                # Golden pass: eval questions from the held-out chunks. Always
                # faithfulness-gated (it IS the measuring stick), and best-effort
                # overall — a transiently failed golden pass must never discard a
                # successful training dataset.
                golden_checkpoint = self._build_checkpoint(
                    input, f"{run_key}-golden" if run_key else ""
                )
                try:
                    (
                        golden_records,
                        golden_dropped,
                        golden_failed,
                    ) = await generate_pairs_with_checkpoint(
                        holdout_chunks,
                        facets,
                        pair_generator,
                        scorer,
                        golden_checkpoint,
                        task_type=input.task_type,
                        guidance=input.guidance,
                        pairs_per_chunk=min(GOLDEN_PAIRS_PER_CHUNK, pairs_per_chunk),
                        faithfulness_enabled=True,
                    )
                except RuntimeError as exc:
                    activity.logger.warning(
                        "Golden eval-set generation failed; dataset proceeds without one: %s",
                        exc,
                    )
                else:
                    activity.logger.info(
                        "Golden set: kept=%d dropped_unfaithful=%d failed_chunks=%d holdout=%d",
                        len(golden_records),
                        golden_dropped,
                        golden_failed,
                        len(holdout_chunks),
                    )

        activity.logger.info(
            "Pair generation: kept=%d dropped_unfaithful=%d failed_chunks=%d "
            "training_chunks=%d holdout_chunks=%d",
            len(records),
            dropped_unfaithful,
            failed_chunks,
            len(training_chunks),
            len(holdout_chunks),
        )

        if not records:
            self._clear_checkpoint(checkpoint)
            self._clear_checkpoint(golden_checkpoint)
            return GenerateSyntheticPairsOutput(pair_count=0, storage_path="")

        # A stable batch id makes the final write idempotent: a crash after the
        # last chunk but before this write is retried to the same key rather than
        # orphaning a partial object.
        batch_id = run_key or str(uuid.uuid4())
        pairs_key = s3_paths.pairs_path(input.tenant_id, input.project_id, batch_id)
        lines = [json.dumps(p, ensure_ascii=False) for p in records]
        s3.put_object(
            Bucket=bucket,
            Key=pairs_key,
            Body="\n".join(lines).encode("utf-8"),
            ContentType="application/jsonl",
        )

        golden_key = ""
        if golden_records:
            golden_key = s3_paths.pairs_path(
                input.tenant_id, input.project_id, f"{batch_id}-golden"
            )
            golden_lines = [json.dumps(p, ensure_ascii=False) for p in golden_records]
            s3.put_object(
                Bucket=bucket,
                Key=golden_key,
                Body="\n".join(golden_lines).encode("utf-8"),
                ContentType="application/jsonl",
            )

        self._clear_checkpoint(checkpoint)
        self._clear_checkpoint(golden_checkpoint)
        return GenerateSyntheticPairsOutput(
            pair_count=len(records),
            storage_path=pairs_key,
            golden_pair_count=len(golden_records),
            golden_storage_path=golden_key,
        )

    def _run_key(self) -> str:
        """Stable-across-retries, unique-across-executions key for this run.

        Temporal keeps workflow-run and activity ids constant across retry
        attempts, so they identify a single logical execution. Returns "" when no
        activity context is available (e.g. direct unit calls), which disables
        checkpointing rather than risking a colliding key.
        """
        try:
            info = activity.info()
            raw = f"{info.workflow_run_id}-{info.activity_id}"
        except Exception:
            return ""
        return re.sub(r"[^A-Za-z0-9._-]", "_", raw)

    def _build_checkpoint(self, input: GenerateSyntheticPairsInput, run_key: str) -> Checkpoint:
        if not self.infra.settings.pair_checkpoint_enabled or not run_key:
            return NullCheckpoint()
        prefix = s3_paths.pairs_checkpoint_prefix(input.tenant_id, input.project_id, run_key)
        return PairCheckpoint(self.infra.s3, self.infra.s3_bucket, prefix)

    def _clear_checkpoint(self, checkpoint: Checkpoint) -> None:
        try:
            checkpoint.clear()
        except Exception as exc:
            activity.logger.warning(
                "Failed to clear pair checkpoint; orphaned objects may remain: %s", exc
            )
