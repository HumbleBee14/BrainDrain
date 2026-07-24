"""Data Studio activities — interactive facet extraction, sample preview,
guidance refinement, and the DB write that persists results to `data_guides`.

Mirrors the pattern in generate_pairs.py: tenant LLM config is resolved from
the DB at activity execution time (never in the workflow, so secrets never
appear in Temporal history), and every generation capability is obtained via
`src.datagen.registry` — never by instantiating an `Llm*` impl directly.
"""

import json
import logging
import uuid
from dataclasses import dataclass

import httpx
from temporalio import activity

from src import s3_paths
from src.activities.generate_pairs import TRANSIENT_GENERATION_ERRORS, apply_faithfulness_gate
from src.backends.chunking_strategy import get as get_chunking_strategy
from src.backends.llm_provider import get as get_llm_provider
from src.datagen.impls import LlmCall
from src.datagen.protocols import Facet, GeneratedPair, RatedSample
from src.datagen.registry import (
    get_facet_extractor,
    get_faithfulness_scorer,
    get_guidance_refiner,
    get_pair_generator,
)
from src.infra import InfraContainer
from src.tenant_config import TenantLlmConfig, get_tenant_llm_config

logger = logging.getLogger("platform.datagen_activities")


def dedupe_facets(facets: list[Facet]) -> list[Facet]:
    """Case-insensitive dedup by label, preserving first-seen order."""
    seen: set[str] = set()
    out: list[Facet] = []
    for facet in facets:
        key = facet.label.strip().lower()
        if key in seen:
            continue
        seen.add(key)
        out.append(facet)
    return out


async def _resolve_tenant_llm(
    infra: InfraContainer, tenant_id: str
) -> tuple[TenantLlmConfig, object]:
    """Fetch tenant LLM config + provider (fail loud if no API key is configured)."""
    settings = infra.settings
    llm_config = await get_tenant_llm_config(
        db=infra.db,
        tenant_id=tenant_id,
        default_api_base_url=settings.llm_api_base_url,
        default_api_key=settings.llm_api_key,
        default_model=settings.llm_model,
        default_max_tokens=settings.llm_max_tokens,
        encryption_key=settings.settings_encryption_key,
    )
    if not llm_config.api_key:
        raise ValueError(
            "LLM API key not configured. Set per-tenant LLM settings via "
            "PUT /api/v1/settings/llm or set APP_LLM_API_KEY environment variable."
        )
    provider = get_llm_provider(settings.llm_provider_backend)
    return llm_config, provider


def _llm_call_closure(
    infra: InfraContainer,
    llm_config: TenantLlmConfig,
    provider,
    http: httpx.AsyncClient,
    temperature: float,
) -> LlmCall:
    """Same closure construction as GeneratePairsActivity — routes every call
    through the circuit breaker with the tenant's resolved provider config.

    `temperature` is bound per closure so the deterministic judge and the
    creative generator can share this factory with different sampling."""

    async def llm_call(prompt: str) -> str:
        return await infra.circuit_breaker.call(
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


async def _fetch_parsed_documents(
    infra: InfraContainer, tenant_id: str, project_id: str
) -> list[tuple[str, str]]:
    """Return (doc_id, full_text) for every parsed document in the project.

    A document whose parsed JSON can't be read is skipped and logged (same
    tolerant-per-item pattern as chunk_text.py) — one bad object must not
    block facet/preview generation for the rest of the project.
    """
    rows = await infra.db.fetch(
        "SELECT id FROM documents WHERE project_id = $1 AND tenant_id = $2 AND status = 'parsed'",
        project_id,
        tenant_id,
    )
    s3 = infra.s3
    bucket = infra.s3_bucket
    texts: list[tuple[str, str]] = []
    for row in rows:
        doc_id = str(row["id"])
        parsed_key = s3_paths.parsed_path(tenant_id, project_id, doc_id)
        try:
            response = s3.get_object(Bucket=bucket, Key=parsed_key)
            parsed_data = json.loads(response["Body"].read())
        except Exception as exc:
            logger.warning("Could not read parsed data for %s: %s", doc_id, exc)
            continue
        text = "\n".join(page.get("text", "") for page in parsed_data.get("pages", []))
        if text.strip():
            texts.append((doc_id, text))
    return texts


# ── Generate facets ──


@dataclass
class GenerateFacetsInput:
    tenant_id: str
    project_id: str
    task_type: str
    guidance: str = ""
    num_facets: int = 8
    existing: list[str] | None = None


@dataclass
class GenerateFacetsOutput:
    facets: list[dict]


class GenerateFacetsActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="generate_facets")
    async def run(self, input: GenerateFacetsInput) -> GenerateFacetsOutput:
        """Extract document-grounded facets for the project's parsed documents."""
        settings = self.infra.settings
        llm_config, provider = await _resolve_tenant_llm(self.infra, input.tenant_id)

        docs = await _fetch_parsed_documents(self.infra, input.tenant_id, input.project_id)
        if not docs:
            return GenerateFacetsOutput(facets=[])

        doc_texts = [text[:6000] for _, text in docs]

        async with httpx.AsyncClient(timeout=120.0) as http:
            llm_call = _llm_call_closure(
                self.infra, llm_config, provider, http, settings.generation_temperature
            )
            extractor = get_facet_extractor(settings, llm_call)
            facets = await extractor.extract(
                doc_texts=doc_texts,
                task_type=input.task_type,
                guidance=input.guidance,
                num_facets=input.num_facets,
                existing=input.existing,
            )

        deduped = dedupe_facets(facets)
        activity.logger.info(
            "Generated %d facets (%d after dedup) from %d documents",
            len(facets),
            len(deduped),
            len(docs),
        )
        return GenerateFacetsOutput(facets=[f.model_dump() for f in deduped])


# ── Generate preview ──


@dataclass
class GeneratePreviewInput:
    tenant_id: str
    project_id: str
    task_type: str
    guidance: str = ""
    facets: list[dict] | None = None
    # Fallback only — the API resolves and clamps this before dispatch. Kept in
    # sync with the Rust DEFAULT_NUM_SAMPLES so the two agree if ever defaulted.
    num_samples: int = 5


@dataclass
class GeneratePreviewOutput:
    preview_samples: list[dict]


class GeneratePreviewActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="generate_preview")
    async def run(self, input: GeneratePreviewInput) -> GeneratePreviewOutput:
        """Generate a small faithfulness-gated sample of pairs for the user to rate."""
        settings = self.infra.settings
        llm_config, provider = await _resolve_tenant_llm(self.infra, input.tenant_id)

        docs = await _fetch_parsed_documents(self.infra, input.tenant_id, input.project_id)
        if not docs:
            return GeneratePreviewOutput(preview_samples=[])

        chunker = get_chunking_strategy(settings.chunking_backend)
        chunks: list[str] = []
        for _, text in docs:
            chunks.extend(chunker.chunk(text, 1500, 200))
        chunks = chunks[: input.num_samples]

        # Only user-kept facets steer generation; a discarded facet must not
        # leak back in via a stale `input.facets` payload. Missing `keep`
        # defaults to kept (matches the DTO default on the Rust side).
        facets = [Facet(**f) for f in input.facets if f.get("keep", True)] if input.facets else []

        async with httpx.AsyncClient(timeout=120.0) as http:
            llm_call = _llm_call_closure(
                self.infra, llm_config, provider, http, settings.generation_temperature
            )
            judge_call = _llm_call_closure(
                self.infra, llm_config, provider, http, settings.judge_temperature
            )
            pair_generator = get_pair_generator(settings, llm_call)
            scorer = get_faithfulness_scorer(settings, judge_call)

            generated: list[GeneratedPair] = []
            facet_ids: dict[int, str | None] = {}
            avoid: list[str] = []
            failed_chunks = 0
            for i, chunk_text_content in enumerate(chunks):
                activity.heartbeat()
                facet = facets[i % len(facets)] if facets else None
                try:
                    pairs = await pair_generator.generate(
                        chunk_text=chunk_text_content[:3000],
                        task_type=input.task_type,
                        guidance=input.guidance,
                        facet=facet,
                        count=1,
                        avoid=list(avoid),
                    )
                except TRANSIENT_GENERATION_ERRORS as exc:
                    failed_chunks += 1
                    activity.logger.warning(
                        "Preview chunk %d generation failed transiently, skipping: %s", i, exc
                    )
                    continue
                for pair in pairs:
                    facet_ids[id(pair)] = facet.id if facet else None
                generated.extend(pairs)
                avoid.extend(p.prompt for p in pairs)

            if failed_chunks > 0 and not generated:
                raise RuntimeError(
                    f"All {failed_chunks} chunk(s) failed preview generation due to "
                    "transient errors; refusing to return an empty preview."
                )

            kept, dropped = await apply_faithfulness_gate(
                generated, scorer, enabled=settings.faithfulness_gate_enabled
            )

        activity.logger.info(
            "Preview generation: generated=%d dropped_unfaithful=%d failed_chunks=%d chunks=%d",
            len(generated),
            dropped,
            failed_chunks,
            len(chunks),
        )

        preview_samples = [
            {
                "id": str(uuid.uuid4()),
                "facet_id": facet_ids.get(id(pair)),
                "prompt": pair.prompt,
                "response": pair.response,
                "rating": None,
            }
            for pair in kept
        ]
        return GeneratePreviewOutput(preview_samples=preview_samples)


# ── Refine guidance ──


@dataclass
class RefineGuidanceInput:
    tenant_id: str
    task_type: str
    guidance: str
    rated: list[dict]


@dataclass
class RefineGuidanceOutput:
    guidance: str
    rationale: str


class RefineGuidanceActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="refine_guidance")
    async def run(self, input: RefineGuidanceInput) -> RefineGuidanceOutput:
        """Run the metaprompter over rated samples to produce improved guidance."""
        settings = self.infra.settings
        llm_config, provider = await _resolve_tenant_llm(self.infra, input.tenant_id)

        rated_samples = [RatedSample(**r) for r in input.rated]

        async with httpx.AsyncClient(timeout=120.0) as http:
            llm_call = _llm_call_closure(
                self.infra, llm_config, provider, http, settings.generation_temperature
            )
            refiner = get_guidance_refiner(settings, llm_call)
            new_guidance, rationale = await refiner.refine(
                task_type=input.task_type,
                current_guidance=input.guidance,
                rated=rated_samples,
            )

        return RefineGuidanceOutput(guidance=new_guidance, rationale=rationale)


# ── Persist results (DB write) ──


@dataclass
class UpdateDataGuideInput:
    tenant_id: str
    data_guide_id: str
    status: str
    facets: list[dict] | None = None
    preview_samples: list[dict] | None = None
    guidance: str | None = None
    refinement_history_entry: dict | None = None
    dataset_id: str | None = None


class UpdateDataGuideActivity:
    """Tenant-scoped DB write of Data Studio results.

    Each calling workflow sets only the columns it produced (facets, preview
    samples, guidance, a refinement_history entry, or a dataset_id) plus the
    terminal status; `refinement_history` is appended to (not overwritten)
    via jsonb concatenation so concurrent history entries are never lost.
    """

    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="update_data_guide")
    async def run(self, input: UpdateDataGuideInput) -> None:
        set_clauses = ["status = $3", "updated_at = now()"]
        params: list = [input.data_guide_id, input.tenant_id, input.status]

        if input.facets is not None:
            params.append(json.dumps(input.facets))
            set_clauses.append(f"facets = ${len(params)}::jsonb")
        if input.preview_samples is not None:
            params.append(json.dumps(input.preview_samples))
            set_clauses.append(f"preview_samples = ${len(params)}::jsonb")
        if input.guidance is not None:
            params.append(input.guidance)
            set_clauses.append(f"guidance = ${len(params)}")
        if input.refinement_history_entry is not None:
            params.append(json.dumps([input.refinement_history_entry]))
            set_clauses.append(f"refinement_history = refinement_history || ${len(params)}::jsonb")
        if input.dataset_id is not None:
            params.append(input.dataset_id)
            set_clauses.append(f"dataset_id = ${len(params)}::uuid")

        query = f"UPDATE data_guides SET {', '.join(set_clauses)} WHERE id = $1 AND tenant_id = $2"
        result = await self.infra.db.execute(query, *params)
        if result == "UPDATE 0":
            raise ValueError(
                f"data_guide not found: {input.data_guide_id} (tenant {input.tenant_id})"
            )
