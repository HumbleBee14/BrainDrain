"""Central library of LLM prompts for the synthetic data-generation pipeline.

Prompts live ONLY here — never inline them at call sites — so wording and
prompt-injection hygiene stay in one small, auditable file.
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

    The base instruction stays the
    primary instruction, and the guidance is appended as a clearly-delimited
    "Special Instructions" data block rather than being spliced into the
    base text.
    """
    return f"""{base}

# Additional Instructions

Beyond the task described above, the operator supplied extra instructions
for this run. Apply both; where they disagree, the extra instructions take
precedence. They are:
<additional_instructions>
{xml_escape(guidance)}
</additional_instructions>
"""


class PromptLibrary:
    """Namespaced collection of prompt builders for the data-gen pipeline."""

    @staticmethod
    def facet_prompt(task_type: str, guidance: str) -> str:
        """Grounded facet-extraction prompt.

        Unlike free topic-tree brainstorming (which lets the model invent an
        arbitrary topic tree), facets here MUST be derived from the actual
        document text provided in the user message — this is
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

        Grounding rules: answerable,
        objective, and strictly grounded in the provided document text.
        """
        base = """You are a **Q&A generation assistant**.

## Task Description
Read the document content provided in the user message and produce
high-quality **query-answer (Q&A) pairs** from it: each pair is one query
plus the answer the document gives to that query.

Write queries the way real users would actually phrase them when hunting
for this information inside a large document collection.

### Important Guidelines
- Only ask what the text settles **definitively** — skip subjective or
  opinion-based questions.
- Never emit a query the provided text cannot answer.
- Ground every answer **strictly in the provided text** — no general
  knowledge, no assumptions — and keep it factually correct and brief.
- Pitch answers at a useful level of detail: not one-word fragments, not
  exhaustive dumps, not vague generalities.
- Mix natural-language questions with short keyword-style search queries.

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
    def subtopic_prompt(task_type: str, guidance: str, num_subtopics: int) -> str:
        """Grounded facet-subtopic expansion prompt.

        Recursive subtopic expansion multiplies generation diversity: rotating
        chunks across facet×subtopic angles instead of a small flat facet list
        prevents samples from clustering on each facet's most obvious phrasing.
        Like facet extraction — and unlike free topic-tree brainstorming —
        subtopics MUST stay grounded in the document excerpt provided in the
        user message.
        """
        base = f"""You are a **facet refinement assistant** for a synthetic data \
generation pipeline.

## Task Description
The task we're generating data for is: **{xml_escape(task_type)}**.

You will be given one **facet** (a theme present in a document) and an excerpt
of the document text it came from. Your job is to break that facet into up to
{num_subtopics} narrower **subtopics** — more specific angles within the facet
that are actually present in the excerpt.

### Important Guidelines
- Every subtopic must be **grounded in the provided document excerpt** — do
  not invent subtopics the text does not support.
- Subtopics must be narrower than the facet, mutually distinct, and each
  usable on its own to steer question generation.
- Keep each subtopic label short (a few words), not a full sentence.
- If the excerpt does not support {num_subtopics} distinct subtopics, return
  fewer — never pad with invented ones.

### Output Format
Return a single JSON object with this exact structure:
```json
{{"subtopics": ["subtopic label 1", "subtopic label 2", ...]}}
```
Use valid JSON only — no extra commentary, explanations, or markdown outside
the JSON object.
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
    def faithfulness_prompt(
        prompt: str,
        response: str,
        source_text: str,
        calibration: list[RatedSample] | None = None,
    ) -> str:
        """Binary faithfulness judge prompt (FaithJudge-style).

        Asks a reasoning-capable judge model for a strict Consistent /
        Hallucinated verdict grounded ONLY in `source_text` — general
        knowledge or assumptions beyond the given text must not be used to
        justify the response.

        `calibration` (optional) renders human-rated examples as a few-shot
        block so the judge's quality bar aligns with the human reviewer's.
        With no examples the output is byte-identical to the uncalibrated
        prompt.
        """
        calibration_block = ""
        if calibration:
            examples = ""
            for i, sample in enumerate(calibration, 1):
                verdict = "acceptable" if sample.looks_good else "below-the-bar"
                examples += f"""<example_{i} verdict="{verdict}">
<prompt>{xml_escape(sample.prompt)}</prompt>
<response>{xml_escape(sample.response)}</response>
</example_{i}>
"""
            calibration_block = f"""
## Calibration Examples
A human reviewer for this project rated these generated examples. Use them
to calibrate how strict to be: responses comparable in quality and grounding
to the acceptable examples should pass; responses with the flaws seen in the
below-the-bar examples should fail.
<calibration_examples>
{examples}</calibration_examples>
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
{calibration_block}
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
