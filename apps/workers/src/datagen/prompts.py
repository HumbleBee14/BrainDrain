# Prompt wording in this file is adapted from the Kiln AI data-gen prompts
# (https://github.com/Kiln-AI/Kiln, MIT), which themselves adapted the
# promptwright library (https://github.com/StacklokLabs/promptwright), which
# adapted the pluto library (https://github.com/redotvideo/pluto).
# promptwright and pluto are licensed under the Apache License 2.0. Any
# modifications here are licensed under this project's license.
"""Central library of LLM prompts for the synthetic data-generation pipeline.

Prompts live ONLY here — never inline them at call sites — so wording,
attribution, and prompt-injection hygiene stay in one small, auditable file.
"""

from html import escape

from src.datagen.protocols import RatedSample


def xml_escape(value: str) -> str:
    """Escape `<`, `>`, `&` so user-supplied text (guidance, ratings,
    source text) can't close out of an XML-style block and re-shape the
    surrounding prompt structure (deliberate or accidental prompt injection).
    """
    return escape(value, quote=False)


def wrap_guidance(base: str, guidance: str) -> str:
    """Wrap a base instruction with additional user guidance.

    Mirrors Kiln's `wrap_task_with_guidance`: the base instruction stays the
    primary instruction, and the guidance is appended as a clearly-delimited
    "Special Instructions" data block rather than being spliced into the
    base text.
    """
    return f"""{base}

# Special Instructions

The above instructions are the original instructions for this task. For this
execution, we've been given additional instructions. Follow both, but
prioritize the additional instructions when they conflict. The additional
instructions are:
<additional_instructions>
{xml_escape(guidance)}
</additional_instructions>
"""


class PromptLibrary:
    """Namespaced collection of prompt builders for the data-gen pipeline."""

    @staticmethod
    def facet_prompt(task_type: str, guidance: str) -> str:
        """Grounded facet-extraction prompt.

        Unlike Kiln's `generate_topic_tree_prompt` (which lets the model
        invent an arbitrary topic tree), facets here MUST be derived from the
        actual document text provided in the user message — this is
        document-grounded synthetic data generation, not free topic
        brainstorming.
        """
        base = f"""You are a **facet extraction assistant** for a synthetic data \
generation pipeline.

## Task Description
The task we're generating data for is: **{xml_escape(task_type)}**.

Your job is to read the document text provided in the user message and
identify a diverse list of **facets** — distinct themes, sections, or angles
that are actually present in that document. Facets are used to spread
generated samples across the real coverage of the document instead of
clustering on one topic.

### Important Guidelines
- Every facet must be **grounded in the document text** — do not invent
  facets that aren't supported by the content you were given.
- Facets should be diverse and non-overlapping where possible.
- Keep each facet label short (a few words), not a full sentence.
- If a list of existing facet labels is provided as `existing`, do not
  repeat or closely paraphrase them — generate new, distinct facets instead.

### Output Format
Return a single JSON object with this exact structure:
```json
{{"facets": ["facet label 1", "facet label 2", ...]}}
```
Use valid JSON only — no extra commentary, explanations, or markdown outside
the JSON object.
"""
        return wrap_guidance(base, guidance) if guidance else base

    @staticmethod
    def qna_grounded_prompt(guidance: str) -> str:
        """Document-grounded Q&A generation prompt.

        Adapted from Kiln's `generate_qna_generation_prompt`: answerable,
        objective, and strictly grounded in the provided document text.
        """
        base = """You are a **Q&A generation assistant**.

## Task Description
Your goal is to generate high-quality **query-answer (Q&A)** pairs from the
document content provided in the user message. A Q&A pair is a query and an
answer to that query.

The queries should reflect **realistic user queries** that someone might ask
when searching a corpus containing this document (among many others).

### Important Guidelines
- Each query must have a **clear, objective answer** based on the document.
  Avoid subjective or opinion-based queries.
- Avoid **unanswerable queries** — every query must be answerable from the
  given text.
- Answers must be **factually correct**, **concise**, and **derived strictly
  from the provided text** — not from general knowledge or assumptions.
- Avoid answers that are too vague, too broad, or too detailed.
- Queries may be phrased as natural questions or as short search-style
  queries.

### Output Format
Return a single JSON object with this exact structure:
```json
{"generated_qna_pairs": [{"query": "...", "answer": "..."}, ...]}
```
Use valid JSON only — no extra commentary, explanations, or markdown outside
the JSON object. Field names must be exactly "query" and "answer".
"""
        return wrap_guidance(base, guidance) if guidance else base

    @staticmethod
    def metaprompter_prompt(task_type: str, current_guidance: str, rated: list[RatedSample]) -> str:
        """Ask the model to improve the current guidance using rated samples.

        Renders the current guidance plus each rated sample (prompt,
        response, looks_good) as XML-escaped data blocks, then asks for
        improved guidance and a one-line rationale as JSON.
        """
        prompt = f"""You are an expert at refining generation guidance for a \
synthetic data pipeline.

## Task Description
The task we're generating data for is: **{xml_escape(task_type)}**.

Below is the current guidance used to steer generation, followed by a set of
previously generated samples that a human has rated as either "Looks Good"
or "Needs Work". Your job is to propose improved guidance that reinforces
what's working and fixes what isn't.

## Current Guidance
<current_guidance>
{xml_escape(current_guidance)}
</current_guidance>

## Rated Samples
"""
        for i, sample in enumerate(rated, 1):
            rating = "Looks Good" if sample.looks_good else "Needs Work"
            prompt += f"""<sample_{i} rating="{rating}">
<prompt>{xml_escape(sample.prompt)}</prompt>
<response>{xml_escape(sample.response)}</response>
</sample_{i}>
"""

        prompt += """
## Your Task
Produce improved guidance that:
- Reinforces patterns present in "Looks Good" samples.
- Corrects whatever is going wrong in "Needs Work" samples.
- Stays concise and actionable — guidance is prepended to future generation
  prompts, not a report.

### Output Format
Return a single JSON object with this exact structure:
```json
{"guidance": "...", "rationale": "..."}
```
`guidance` is the complete replacement guidance text. `rationale` is a
single sentence explaining what changed and why. Use valid JSON only — no
extra commentary, explanations, or markdown outside the JSON object.
"""
        return prompt

    @staticmethod
    def faithfulness_prompt(prompt: str, response: str, source_text: str) -> str:
        """Binary faithfulness judge prompt (FaithJudge-style).

        Asks a reasoning-capable judge model for a strict Consistent /
        Hallucinated verdict grounded ONLY in `source_text` — general
        knowledge or assumptions beyond the given text must not be used to
        justify the response.
        """
        return f"""You are a **faithfulness judge** for a retrieval-augmented \
generation pipeline.

## Task Description
You will be given a source text, a query, and a response that was generated
using only that source text. Your job is to judge whether the response is
**Consistent** with the source text or **Hallucinated**.

- **Consistent**: every factual claim in the response is directly supported
  by the source text.
- **Hallucinated**: the response contains any claim that is not supported
  by, or contradicts, the source text — including claims that may be true
  in general but are not present in this specific source text.

Judge strictly and only against the source text below. Do not use
outside/general knowledge to excuse an unsupported claim.

## Source Text
<source_text>
{xml_escape(source_text)}
</source_text>

## Query
<query>
{xml_escape(prompt)}
</query>

## Response to Judge
<response>
{xml_escape(response)}
</response>

### Output Format
Return a single JSON object with this exact structure:
```json
{{"consistent": true|false, "score": 0.0-1.0, "reason": "..."}}
```
`consistent` is the binary Consistent/Hallucinated verdict (true =
Consistent). `score` is your confidence that the response is faithful, from
0.0 (fully hallucinated) to 1.0 (fully consistent). `reason` is a brief
explanation citing which claim(s) are or are not supported. Use valid JSON
only — no extra commentary, explanations, or markdown outside the JSON
object.
"""
