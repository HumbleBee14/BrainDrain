"""Synthetic pair generation activity — creates instruction/response pairs from chunks.

Uses a configurable LLM API (OpenAI-compatible format) to generate
training data from document chunks. LLM provider config is resolved
per-tenant from the database at execution time. Structured generation
(PairGenerator), facet-diversity distribution, and a faithfulness
quality-gate (FaithfulnessScorer) are provided via src.datagen.registry.
"""

import json
import logging
import uuid
from dataclasses import dataclass

import httpx
from temporalio import activity

from src import s3_paths
from src.backends.llm_provider import get as get_llm_provider
from src.circuit_breaker import CircuitBreakerOpen
from src.datagen.protocols import Facet, FaithfulnessScorer, GeneratedPair, PairGenerator
from src.datagen.registry import get_faithfulness_scorer, get_pair_generator
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


def clamp_pairs_per_chunk(value: int) -> int:
    """Bound pairs_per_chunk to a sane range to cap generation fan-out."""
    return max(MIN_PAIRS_PER_CHUNK, min(value, MAX_PAIRS_PER_CHUNK))


@dataclass
class GenerateSyntheticPairsInput:
    tenant_id: str
    project_id: str
    chunks_storage_path: str
    task_type: str
    pairs_per_chunk: int = 5
    guidance: str = ""
    facets: list[dict] | None = None


@dataclass
class GenerateSyntheticPairsOutput:
    pair_count: int
    storage_path: str


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


async def generate_pairs_for_chunks(
    chunks: list[dict],
    facets: list[Facet],
    pair_generator: PairGenerator,
    *,
    task_type: str,
    guidance: str,
    pairs_per_chunk: int,
) -> tuple[list[GeneratedPair], dict[int, tuple[str | None, str | None]], int]:
    """Generate pairs for every eligible chunk, tolerating transient per-chunk errors.

    A single chunk hitting a timeout, rate limit, or transport error must not
    discard chunks that already succeeded. Only `TRANSIENT_GENERATION_ERRORS`
    are swallowed here; a `ValueError` from the generator (malformed/missing
    LLM output) indicates a real bug and always propagates.

    Raises if every attempted chunk failed and no pairs were produced — that's
    a real failure, not an empty-input case.
    """
    generated: list[GeneratedPair] = []
    pair_meta: dict[int, tuple[str | None, str | None]] = {}
    avoid: list[str] = []
    failed_chunks = 0

    for i, chunk in enumerate(chunks):
        activity.heartbeat()
        chunk_text = chunk.get("text", "")
        if len(chunk_text) < 50:
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

        for pair in pairs:
            pair_meta[id(pair)] = (chunk.get("doc_id"), chunk.get("chunk_id"))
        generated.extend(pairs)
        avoid.extend(p.prompt for p in pairs)

    if failed_chunks > 0 and not generated:
        raise RuntimeError(
            f"All {failed_chunks} chunk(s) failed synthetic pair generation due to "
            "transient errors; refusing to return an empty dataset."
        )

    return generated, pair_meta, failed_chunks


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
        )

        if not llm_config.api_key:
            raise ValueError(
                "LLM API key not configured. Set per-tenant LLM settings via "
                "PUT /api/v1/settings/llm or set APP_LLM_API_KEY environment variable."
            )

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

        # Track source chunk metadata per pair by identity — apply_faithfulness_gate
        # operates on plain GeneratedPair objects, so doc/chunk ids ride alongside
        # rather than through the gate itself.
        # Only user-kept facets steer generation; a discarded facet must not
        # leak back in via a stale `input.facets` payload. Missing `keep`
        # defaults to kept (matches the DTO default on the Rust side).
        facets = [Facet(**f) for f in input.facets if f.get("keep", True)] if input.facets else []

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
            scorer = get_faithfulness_scorer(settings, make_llm_call(settings.judge_temperature))

            pairs_per_chunk = clamp_pairs_per_chunk(input.pairs_per_chunk)
            generated, pair_meta, failed_chunks = await generate_pairs_for_chunks(
                chunks,
                facets,
                pair_generator,
                task_type=input.task_type,
                guidance=input.guidance,
                pairs_per_chunk=pairs_per_chunk,
            )

        kept, dropped_unfaithful = await apply_faithfulness_gate(
            generated, scorer, enabled=settings.faithfulness_gate_enabled
        )

        activity.logger.info(
            "Pair generation: generated=%d dropped_unfaithful=%d failed_chunks=%d chunks=%d",
            len(generated),
            dropped_unfaithful,
            failed_chunks,
            len(chunks),
        )

        if not kept:
            return GenerateSyntheticPairsOutput(pair_count=0, storage_path="")

        all_pairs = [
            {
                "id": str(uuid.uuid4()),
                "doc_id": pair_meta[id(pair)][0],
                "chunk_id": pair_meta[id(pair)][1],
                "task_type": input.task_type,
                "instruction": pair.prompt,
                "response": pair.response,
                "source_text": pair.source_text[:500],
            }
            for pair in kept
        ]

        # Upload pairs as JSONL
        batch_id = str(uuid.uuid4())
        pairs_key = s3_paths.pairs_path(input.tenant_id, input.project_id, batch_id)
        lines = [json.dumps(p, ensure_ascii=False) for p in all_pairs]
        s3.put_object(
            Bucket=bucket,
            Key=pairs_key,
            Body="\n".join(lines).encode("utf-8"),
            ContentType="application/jsonl",
        )

        return GenerateSyntheticPairsOutput(pair_count=len(all_pairs), storage_path=pairs_key)
