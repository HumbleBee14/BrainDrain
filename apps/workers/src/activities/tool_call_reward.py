"""Verifiable GRPO reward for tool-call completions.

Instead of asking an LLM judge whether a generated tool call "looks right",
the reward is computed deterministically from the completion itself: does it
contain a parsable tool call, does the call name exist in the record's tool
schema, do the arguments satisfy that tool's parameter schema, and does it
match the reference call from the gold trajectory. Pure functions — no model,
no network — so the reward is reproducible and cheap.

Reward scale is [-1.0, 1.0], matching the LLM judge's `score_reasoning`:

    -1.0  no parsable tool call in the completion
     0.0  parsable call, but the tool name is not in the schema
     0.4  name valid, arguments do not satisfy the parameter schema
     0.7  name and arguments valid
     1.0  fully valid and the name matches the reference call
          (the reference component is granted when no reference exists)

Multiple calls in one completion are scored individually and averaged.
"""

from __future__ import annotations

import json
import re

_TOOL_CALL_BLOCK = re.compile(r"<tool_call>\s*(.*?)\s*</tool_call>", re.DOTALL)

_NAME_WEIGHT = 0.4
_ARGS_WEIGHT = 0.3
_REFERENCE_WEIGHT = 0.3


def extract_tool_calls(text: str) -> list[dict]:
    """Extract normalized tool calls from a generated completion.

    Looks for ``<tool_call>{...}</tool_call>`` blocks (the ChatML tool-call
    convention the fallback template trains); when none are present, tries the
    whole completion as a single JSON object. Each parsable call is normalized
    to ``{"name": str, "arguments": dict | None}`` — ``None`` arguments mean
    the call carried arguments that could not be parsed into an object.
    Calls without a usable string name are dropped.
    """
    candidates = _TOOL_CALL_BLOCK.findall(text)
    if not candidates:
        stripped = text.strip()
        if stripped.startswith("{"):
            candidates = [stripped]

    calls = []
    for raw in candidates:
        try:
            data = json.loads(raw)
        except (json.JSONDecodeError, TypeError):
            continue
        normalized = _normalize_call(data)
        if normalized is not None:
            calls.append(normalized)
    return calls


def _normalize_call(data) -> dict | None:
    """Normalize one call object (bare or OpenAI ``function``-wrapped)."""
    if not isinstance(data, dict):
        return None
    fn = data.get("function") if isinstance(data.get("function"), dict) else data
    name = fn.get("name")
    if not isinstance(name, str) or not name.strip():
        return None
    arguments = fn.get("arguments")
    if isinstance(arguments, str):
        try:
            arguments = json.loads(arguments)
        except json.JSONDecodeError:
            arguments = None
    if arguments is None:
        arguments = {} if "arguments" not in fn else arguments
    if not isinstance(arguments, dict):
        arguments = None
    return {"name": name, "arguments": arguments}


def _schema_map(tools: list) -> dict[str, dict]:
    """Map tool name → parameter schema from an OpenAI-format tools list."""
    schemas = {}
    for tool in tools or []:
        if not isinstance(tool, dict):
            continue
        fn = tool.get("function") if isinstance(tool.get("function"), dict) else tool
        name = fn.get("name")
        if isinstance(name, str) and name.strip():
            params = fn.get("parameters")
            schemas[name] = params if isinstance(params, dict) else {}
    return schemas


def _arguments_satisfy_schema(arguments: dict | None, params: dict) -> bool:
    """Shallow parameter-schema check: required keys present, no unknown keys.

    A tool without a declared parameter schema accepts any parsable arguments
    object; declared ``properties`` bound the accepted keys.
    """
    if arguments is None:
        return False
    if not params:
        return True
    required = params.get("required")
    if isinstance(required, list) and not set(required) <= set(arguments):
        return False
    properties = params.get("properties")
    if isinstance(properties, dict) and properties:
        return set(arguments) <= set(properties)
    return True


def reference_call_names(reference_calls: list) -> set[str]:
    """Collect tool names from a gold trajectory's ``tool_calls`` list."""
    names = set()
    for call in reference_calls or []:
        normalized = _normalize_call(call)
        if normalized is not None:
            names.add(normalized["name"])
    return names


def score_tool_call_completion(
    completion: str,
    *,
    tools: list,
    reference_calls: list,
) -> float:
    """Score a completion against the record's tool schema and reference call.

    See the module docstring for the rubric. With an empty ``tools`` schema
    the name and argument components are granted (nothing to validate
    against), so the score is driven by parse success and reference match.
    """
    calls = extract_tool_calls(completion)
    if not calls:
        return -1.0

    schemas = _schema_map(tools)
    ref_names = reference_call_names(reference_calls)

    total = 0.0
    for call in calls:
        score = 0.0
        name_valid = not schemas or call["name"] in schemas
        if name_valid:
            score += _NAME_WEIGHT
            params = schemas.get(call["name"], {})
            if _arguments_satisfy_schema(call["arguments"], params):
                score += _ARGS_WEIGHT
            if not ref_names or call["name"] in ref_names:
                score += _REFERENCE_WEIGHT
        total += score
    return max(-1.0, min(1.0, total / len(calls)))
