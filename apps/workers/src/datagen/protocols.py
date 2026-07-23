"""Swappable data-generation abstractions. Impls selected via registry."""

from typing import Protocol, runtime_checkable

from pydantic import BaseModel, Field


class Facet(BaseModel):
    id: str
    label: str
    source_doc_id: str | None = None
    keep: bool = True


class GeneratedPair(BaseModel):
    prompt: str
    response: str
    facet_id: str | None = None
    source_text: str = ""


class RatedSample(BaseModel):
    prompt: str
    response: str
    looks_good: bool


class FaithfulnessVerdict(BaseModel):
    consistent: bool
    score: float = Field(ge=0.0, le=1.0)
    reason: str = ""


@runtime_checkable
class FacetExtractor(Protocol):
    async def extract(
        self,
        *,
        doc_texts: list[str],
        task_type: str,
        guidance: str,
        num_facets: int,
        existing: list[str] | None = None,
    ) -> list[Facet]: ...


@runtime_checkable
class FacetExpander(Protocol):
    async def expand(
        self,
        *,
        facet: Facet,
        doc_sample: str,
        task_type: str,
        guidance: str,
        num_subtopics: int,
    ) -> list[str]: ...


@runtime_checkable
class PairGenerator(Protocol):
    async def generate(
        self,
        *,
        chunk_text: str,
        task_type: str,
        guidance: str,
        facet: Facet | None,
        count: int,
        avoid: list[str] | None = None,
    ) -> list[GeneratedPair]: ...


@runtime_checkable
class GuidanceRefiner(Protocol):
    async def refine(
        self, *, task_type: str, current_guidance: str, rated: list[RatedSample]
    ) -> tuple[str, str]:  # (new_guidance, rationale)
        ...


@runtime_checkable
class FaithfulnessScorer(Protocol):
    async def score(self, *, pair: GeneratedPair, source_text: str) -> FaithfulnessVerdict: ...
