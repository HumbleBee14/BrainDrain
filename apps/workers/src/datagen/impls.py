"""Default LLM-backed implementations of the data-gen protocols.

Each impl takes an injected `llm_call` coroutine (prompt -> raw text response)
so it stays decoupled from the concrete provider/transport and unit-testable
with a fake. Production wiring (a later task) passes a closure that calls the
real tenant-configured provider (src.backends.llm_provider + src.tenant_config).

All LLM responses are parsed strictly — malformed JSON or missing keys raise
rather than silently falling back to empty/default values (fail loud).
"""

import json
import uuid
from collections.abc import Awaitable, Callable

from src.datagen.prompts import PromptLibrary, xml_escape
from src.datagen.protocols import Facet, FaithfulnessVerdict, GeneratedPair, RatedSample

LlmCall = Callable[[str], Awaitable[str]]

# Per-verdict cap so a long rating history can't blow up the judge prompt.
MAX_CALIBRATION_PER_VERDICT = 4


def select_calibration_examples(rated: list[dict] | None) -> list[RatedSample]:
    """Cap and clean human ratings used to calibrate the faithfulness judge.

    Keeps at most MAX_CALIBRATION_PER_VERDICT accepted and as many rejected
    examples, preferring the most recent, skipping entries with an empty
    prompt or response. Original order is preserved in the result.
    """
    if not rated:
        return []
    good = bad = 0
    selected: list[RatedSample] = []
    for entry in reversed(rated):
        if not isinstance(entry, dict):
            continue
        prompt = entry.get("prompt")
        response = entry.get("response")
        looks_good = entry.get("looks_good")
        if not isinstance(prompt, str) or not prompt.strip():
            continue
        if not isinstance(response, str) or not response.strip():
            continue
        if not isinstance(looks_good, bool):
            continue
        if looks_good:
            if good >= MAX_CALIBRATION_PER_VERDICT:
                continue
            good += 1
        else:
            if bad >= MAX_CALIBRATION_PER_VERDICT:
                continue
            bad += 1
        selected.append(RatedSample(prompt=prompt, response=response, looks_good=looks_good))
    selected.reverse()
    return selected


def _parse_json_object(raw: str, *, required_keys: tuple[str, ...]) -> dict:
    """Parse `raw` as a JSON object containing every key in `required_keys`.

    Raises ValueError (not a silent default) on malformed JSON or a missing key.
    """
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise ValueError(f"LLM response was not valid JSON: {raw!r}") from exc

    if not isinstance(data, dict):
        raise ValueError(f"LLM response was not a JSON object: {raw!r}")

    missing = [key for key in required_keys if key not in data]
    if missing:
        raise ValueError(f"LLM response missing required keys {missing}: {raw!r}")

    return data


class LlmPairGenerator:
    """Default PairGenerator: renders the grounded Q&A prompt and parses pairs."""

    def __init__(self, llm_call: LlmCall):
        self.llm_call = llm_call

    async def generate(
        self,
        *,
        chunk_text: str,
        task_type: str,
        guidance: str,
        facet: Facet | None,
        count: int,
        avoid: list[str] | None = None,
    ) -> list[GeneratedPair]:
        prompt = PromptLibrary.qna_grounded_prompt(guidance)
        avoid_text = "\n".join(xml_escape(item) for item in avoid) if avoid else "(none)"
        message = f"""{prompt}

## Document Text
<document_text>
{xml_escape(chunk_text)}
</document_text>

## Generation Parameters
- Number of Q&A pairs to generate: {count}
- Facet to focus on: {facet.label if facet else "(none — cover the document generally)"}
- Avoid generating queries similar to:
<avoid_questions>
{avoid_text}
</avoid_questions>
"""
        raw = await self.llm_call(message)
        data = _parse_json_object(raw, required_keys=("generated_qna_pairs",))
        raw_pairs = data["generated_qna_pairs"]
        if not isinstance(raw_pairs, list):
            raise ValueError(f"'generated_qna_pairs' was not a list: {raw!r}")

        pairs: list[GeneratedPair] = []
        for item in raw_pairs:
            if not isinstance(item, dict) or "query" not in item or "answer" not in item:
                raise ValueError(f"Malformed Q&A pair entry: {item!r}")
            pairs.append(
                GeneratedPair(
                    prompt=item["query"],
                    response=item["answer"],
                    source_text=chunk_text,
                    facet_id=facet.id if facet else None,
                )
            )
        return pairs


class LlmFacetExtractor:
    """Default FacetExtractor: renders the facet-extraction prompt and parses labels."""

    def __init__(self, llm_call: LlmCall):
        self.llm_call = llm_call

    async def extract(
        self,
        *,
        doc_texts: list[str],
        task_type: str,
        guidance: str,
        num_facets: int,
        existing: list[str] | None = None,
    ) -> list[Facet]:
        prompt = PromptLibrary.facet_prompt(task_type, guidance)
        document_text = "\n".join(xml_escape(text) for text in doc_texts)
        existing_text = "\n".join(xml_escape(label) for label in existing) if existing else "(none)"
        message = f"""{prompt}

## Document Text
<document_text>
{document_text}
</document_text>

## Generation Parameters
- Number of facets to extract: {num_facets}
- Existing facet labels (do not repeat):
<existing_facets>
{existing_text}
</existing_facets>
"""
        raw = await self.llm_call(message)
        data = _parse_json_object(raw, required_keys=("facets",))
        labels = data["facets"]
        if not isinstance(labels, list):
            raise ValueError(f"'facets' was not a list: {raw!r}")

        facets: list[Facet] = []
        for label in labels:
            if not isinstance(label, str):
                raise ValueError(f"Malformed facet label: {label!r}")
            facets.append(Facet(id=str(uuid.uuid4()), label=label))
        return facets


class LlmFacetExpander:
    """Default FacetExpander: breaks one facet into grounded subtopics."""

    def __init__(self, llm_call: LlmCall):
        self.llm_call = llm_call

    async def expand(
        self,
        *,
        facet: Facet,
        doc_sample: str,
        task_type: str,
        guidance: str,
        num_subtopics: int,
    ) -> list[str]:
        prompt = PromptLibrary.subtopic_prompt(task_type, guidance, num_subtopics)
        message = f"""{prompt}

## Facet To Expand
<facet>
{xml_escape(facet.label)}
</facet>

## Document Excerpt
<document_text>
{xml_escape(doc_sample)}
</document_text>
"""
        raw = await self.llm_call(message)
        data = _parse_json_object(raw, required_keys=("subtopics",))
        subtopics = data["subtopics"]
        if not isinstance(subtopics, list):
            raise ValueError(f"'subtopics' was not a list: {raw!r}")
        labels: list[str] = []
        for label in subtopics:
            if not isinstance(label, str):
                raise ValueError(f"Malformed subtopic label: {label!r}")
            if label.strip():
                labels.append(label.strip())
        # The prompt allows returning fewer; never accept more than asked for.
        return labels[:num_subtopics]


class LlmGuidanceRefiner:
    """Default GuidanceRefiner: renders the metaprompter prompt and parses the update."""

    def __init__(self, llm_call: LlmCall):
        self.llm_call = llm_call

    async def refine(
        self, *, task_type: str, current_guidance: str, rated: list[RatedSample]
    ) -> tuple[str, str]:
        prompt = PromptLibrary.metaprompter_prompt(task_type, current_guidance, rated)
        raw = await self.llm_call(prompt)
        data = _parse_json_object(raw, required_keys=("guidance", "rationale"))
        return data["guidance"], data["rationale"]


class LlmFaithfulnessScorer:
    """Default FaithfulnessScorer: renders the faithfulness-judge prompt and parses the verdict.

    `calibration` (optional raw `{prompt, response, looks_good}` dicts) is
    capped/cleaned once here and rendered into every judge prompt as few-shot
    examples of the human reviewer's quality bar.
    """

    def __init__(self, llm_call: LlmCall, calibration: list[dict] | None = None):
        self.llm_call = llm_call
        self.calibration = select_calibration_examples(calibration)

    async def score(self, *, pair: GeneratedPair, source_text: str) -> FaithfulnessVerdict:
        prompt = PromptLibrary.faithfulness_prompt(
            pair.prompt, pair.response, source_text, calibration=self.calibration
        )
        raw = await self.llm_call(prompt)
        data = _parse_json_object(raw, required_keys=("consistent", "score", "reason"))
        return FaithfulnessVerdict(
            consistent=data["consistent"], score=data["score"], reason=data["reason"]
        )
