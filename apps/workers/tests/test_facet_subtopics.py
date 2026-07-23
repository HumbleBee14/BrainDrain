"""Tests for facet-subtopic expansion (topic-tree × facet diversity).

Facets are expanded into grounded subtopics and flattened into rotation
"angles" (facet first, then facet — subtopic variants). A failed or empty
expansion must degrade to the base facet, never fail generation.
"""

import pytest

from src.activities.generate_pairs import (
    MAX_SUBTOPICS_PER_FACET,
    doc_sample_for_expansion,
    facets_to_angles,
)
from src.datagen.impls import LlmFacetExpander
from src.datagen.protocols import Facet


def _facet(fid: str, label: str) -> Facet:
    return Facet(id=fid, label=label)


class TestFacetsToAngles:
    def test_base_facet_always_first_then_subtopics(self):
        facets = [_facet("f1", "Billing")]
        angles = facets_to_angles(facets, {"f1": ["refunds", "invoices"]})
        assert [a.label for a in angles] == [
            "Billing",
            "Billing — refunds",
            "Billing — invoices",
        ]
        assert [a.id for a in angles] == ["f1", "f1.0", "f1.1"]

    def test_facet_without_expansion_kept_unexpanded(self):
        facets = [_facet("f1", "Billing"), _facet("f2", "Security")]
        angles = facets_to_angles(facets, {"f1": ["refunds"]})
        assert [a.label for a in angles] == ["Billing", "Billing — refunds", "Security"]

    def test_empty_expansion_map_is_identity(self):
        facets = [_facet("f1", "A"), _facet("f2", "B")]
        angles = facets_to_angles(facets, {})
        assert angles == facets

    def test_provenance_preserved(self):
        facets = [Facet(id="f1", label="Billing", source_doc_id="doc-9")]
        angles = facets_to_angles(facets, {"f1": ["refunds"]})
        assert angles[1].source_doc_id == "doc-9"


class TestDocSampleForExpansion:
    def test_empty_chunks(self):
        assert doc_sample_for_expansion([]) == ""

    def test_respects_limit(self):
        chunks = [{"text": "x" * 3000} for _ in range(10)]
        sample = doc_sample_for_expansion(chunks, limit=4000)
        assert len(sample) <= 4000 + 10  # joins add separators

    def test_samples_across_document_not_just_head(self):
        chunks = [{"text": f"CHUNK{i} " + "pad " * 50} for i in range(40)]
        sample = doc_sample_for_expansion(chunks, limit=4000)
        # step = 40 // 8 = 5 → includes chunks beyond the head
        assert "CHUNK0" in sample
        assert "CHUNK5" in sample


class TestLlmFacetExpander:
    @pytest.mark.asyncio
    async def test_parses_subtopics(self):
        async def llm(prompt: str) -> str:
            return '{"subtopics": ["refund policy", "invoice history", "  ", "late fees"]}'

        out = await LlmFacetExpander(llm).expand(
            facet=_facet("f1", "Billing"),
            doc_sample="doc text",
            task_type="qa",
            guidance="",
            num_subtopics=3,
        )
        # Blank entries dropped; capped at num_subtopics.
        assert out == ["refund policy", "invoice history", "late fees"]

    @pytest.mark.asyncio
    async def test_never_returns_more_than_asked(self):
        async def llm(prompt: str) -> str:
            return '{"subtopics": ["a", "b", "c", "d", "e"]}'

        out = await LlmFacetExpander(llm).expand(
            facet=_facet("f1", "X"),
            doc_sample="t",
            task_type="qa",
            guidance="",
            num_subtopics=2,
        )
        assert out == ["a", "b"]

    @pytest.mark.asyncio
    async def test_malformed_response_raises(self):
        async def llm(prompt: str) -> str:
            return '{"wrong_key": []}'

        with pytest.raises(ValueError):
            await LlmFacetExpander(llm).expand(
                facet=_facet("f1", "X"),
                doc_sample="t",
                task_type="qa",
                guidance="",
                num_subtopics=2,
            )

    @pytest.mark.asyncio
    async def test_non_string_subtopic_raises(self):
        async def llm(prompt: str) -> str:
            return '{"subtopics": ["ok", 42]}'

        with pytest.raises(ValueError):
            await LlmFacetExpander(llm).expand(
                facet=_facet("f1", "X"),
                doc_sample="t",
                task_type="qa",
                guidance="",
                num_subtopics=5,
            )


def test_cap_constant_is_sane():
    assert 1 <= MAX_SUBTOPICS_PER_FACET <= 10
