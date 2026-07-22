import json
from types import SimpleNamespace

import pytest

from src.datagen.impls import (
    LlmFacetExtractor,
    LlmFaithfulnessScorer,
    LlmGuidanceRefiner,
    LlmPairGenerator,
)
from src.datagen.protocols import Facet, FaithfulnessVerdict, GeneratedPair, RatedSample
from src.datagen.registry import (
    get_facet_extractor,
    get_faithfulness_scorer,
    get_guidance_refiner,
    get_pair_generator,
)


async def _dummy_llm_call(prompt: str) -> str:
    return "{}"


def test_shared_types_construct_and_default():
    f = Facet(id="f1", label="Billing", source_doc_id=None)
    assert f.keep is True
    p = GeneratedPair(prompt="q", response="a", facet_id="f1", source_text="src")
    assert p.facet_id == "f1"
    v = FaithfulnessVerdict(consistent=False, score=0.1, reason="unsupported")
    assert v.consistent is False
    r = RatedSample(prompt="q", response="a", looks_good=True)
    assert r.looks_good is True


@pytest.mark.asyncio
async def test_pair_generator_parses_structured_output():
    async def fake_llm(prompt: str) -> str:
        return json.dumps({"generated_qna_pairs": [{"query": "Q1", "answer": "A1"}]})

    gen = LlmPairGenerator(llm_call=fake_llm)
    pairs = await gen.generate(
        chunk_text="src", task_type="question_answering", guidance="", facet=None, count=1
    )
    assert pairs[0].prompt == "Q1" and pairs[0].response == "A1" and pairs[0].source_text == "src"


@pytest.mark.asyncio
async def test_faithfulness_scorer_maps_verdict():
    async def fake_llm(prompt: str) -> str:
        return json.dumps({"consistent": False, "score": 0.2, "reason": "unsupported"})

    scorer = LlmFaithfulnessScorer(llm_call=fake_llm)
    v = await scorer.score(
        pair=GeneratedPair(prompt="q", response="a", source_text="s"), source_text="s"
    )
    assert v.consistent is False and v.score == 0.2


@pytest.mark.asyncio
async def test_pair_generator_raises_on_malformed_json():
    async def fake_llm(prompt: str) -> str:
        return "not valid json"

    gen = LlmPairGenerator(llm_call=fake_llm)
    with pytest.raises(ValueError):
        await gen.generate(
            chunk_text="src", task_type="question_answering", guidance="", facet=None, count=1
        )


@pytest.mark.asyncio
async def test_pair_generator_raises_on_missing_required_key():
    async def fake_llm(prompt: str) -> str:
        return json.dumps({"unexpected_key": []})

    gen = LlmPairGenerator(llm_call=fake_llm)
    with pytest.raises(ValueError):
        await gen.generate(
            chunk_text="src", task_type="question_answering", guidance="", facet=None, count=1
        )


class TestGetFacetExtractor:
    def test_returns_llm_impl_when_backend_is_llm(self):
        settings = SimpleNamespace(datagen_facet_backend="llm")
        extractor = get_facet_extractor(settings, llm_call=_dummy_llm_call)
        assert isinstance(extractor, LlmFacetExtractor)

    def test_raises_on_unknown_backend(self):
        settings = SimpleNamespace(datagen_facet_backend="rule_based")
        with pytest.raises(ValueError):
            get_facet_extractor(settings, llm_call=_dummy_llm_call)


class TestGetPairGenerator:
    def test_returns_llm_impl_when_backend_is_llm(self):
        settings = SimpleNamespace(datagen_pair_backend="llm")
        generator = get_pair_generator(settings, llm_call=_dummy_llm_call)
        assert isinstance(generator, LlmPairGenerator)

    def test_raises_on_unknown_backend(self):
        settings = SimpleNamespace(datagen_pair_backend="rule_based")
        with pytest.raises(ValueError):
            get_pair_generator(settings, llm_call=_dummy_llm_call)


class TestGetGuidanceRefiner:
    def test_returns_llm_impl_when_backend_is_llm(self):
        settings = SimpleNamespace(datagen_refiner_backend="llm")
        refiner = get_guidance_refiner(settings, llm_call=_dummy_llm_call)
        assert isinstance(refiner, LlmGuidanceRefiner)

    def test_raises_on_unknown_backend(self):
        settings = SimpleNamespace(datagen_refiner_backend="rule_based")
        with pytest.raises(ValueError):
            get_guidance_refiner(settings, llm_call=_dummy_llm_call)


class TestGetFaithfulnessScorer:
    def test_returns_llm_impl_when_backend_is_llm(self):
        settings = SimpleNamespace(datagen_faithfulness_backend="llm")
        scorer = get_faithfulness_scorer(settings, llm_call=_dummy_llm_call)
        assert isinstance(scorer, LlmFaithfulnessScorer)

    def test_raises_on_unknown_backend(self):
        settings = SimpleNamespace(datagen_faithfulness_backend="rule_based")
        with pytest.raises(ValueError):
            get_faithfulness_scorer(settings, llm_call=_dummy_llm_call)
