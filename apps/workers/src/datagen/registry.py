"""Config-selected registry for the data-gen protocol implementations.

Selection is driven by `settings.datagen_*_backend` (see src/config.py).
Currently only the "llm" backend is registered; the registry seam exists so
future backends (e.g. rule-based facet extraction) can be added without
touching call sites.
"""

from src.config import WorkerSettings
from src.datagen.impls import (
    LlmCall,
    LlmFacetExtractor,
    LlmFaithfulnessScorer,
    LlmGuidanceRefiner,
    LlmPairGenerator,
)
from src.datagen.protocols import FacetExtractor, FaithfulnessScorer, GuidanceRefiner, PairGenerator


def get_facet_extractor(settings: WorkerSettings, llm_call: LlmCall) -> FacetExtractor:
    if settings.datagen_facet_backend == "llm":
        return LlmFacetExtractor(llm_call=llm_call)
    raise ValueError(f"unknown datagen_facet_backend: {settings.datagen_facet_backend}")


def get_pair_generator(settings: WorkerSettings, llm_call: LlmCall) -> PairGenerator:
    if settings.datagen_pair_backend == "llm":
        return LlmPairGenerator(llm_call=llm_call)
    raise ValueError(f"unknown datagen_pair_backend: {settings.datagen_pair_backend}")


def get_guidance_refiner(settings: WorkerSettings, llm_call: LlmCall) -> GuidanceRefiner:
    if settings.datagen_refiner_backend == "llm":
        return LlmGuidanceRefiner(llm_call=llm_call)
    raise ValueError(f"unknown datagen_refiner_backend: {settings.datagen_refiner_backend}")


def get_faithfulness_scorer(settings: WorkerSettings, llm_call: LlmCall) -> FaithfulnessScorer:
    if settings.datagen_faithfulness_backend == "llm":
        return LlmFaithfulnessScorer(llm_call=llm_call)
    raise ValueError(
        f"unknown datagen_faithfulness_backend: {settings.datagen_faithfulness_backend}"
    )
