"""Unified LLM-as-Judge module used by both training and evaluation.

Provides a single JudgeLLM class that handles:
  - Domain scoring (accuracy, completeness, faithfulness)
  - A/B comparison (blind pairwise)
  - Correctness checking (for benchmarks)
  - Response quality scoring (for DPO pair creation)
  - Reasoning reward scoring (for GRPO)
  - Refusal classification (for safety checks)

All judge calls go through the same OpenAI-compatible API client,
with consistent error handling and heuristic fallbacks.
"""

import json
import logging
from typing import Protocol

import httpx

logger = logging.getLogger("platform.judge")


class LLMJudge(Protocol):
    """Protocol for LLM judge implementations.

    Any class implementing these methods can be used as a judge
    in both training and evaluation pipelines.
    """

    def score_response(self, prompt: str, response: str) -> float:
        """Score a response quality (1-10). Used by DPO pair creation."""
        ...

    def score_reasoning(self, completion: str) -> float:
        """Score reasoning quality, normalized to [-1, 1]. Used by GRPO."""
        ...

    def score_domain(self, prompt: str, generated: str, expected: str) -> dict:
        """Score on accuracy/completeness/faithfulness (1-5 each)."""
        ...

    def compare_ab(self, prompt: str, response_a: str, response_b: str) -> str:
        """Blind A/B comparison. Returns 'A', 'B', or 'tie'."""
        ...

    def check_correctness(self, answer: str, expected: str) -> bool:
        """Check if an answer matches the expected answer."""
        ...


class OpenAICompatibleJudge:
    """LLM-as-Judge backed by any OpenAI-compatible API.

    Handles all judge operations with consistent error handling and
    heuristic fallbacks when the API is unavailable.
    """

    def __init__(self, api_base: str, api_key: str, model: str):
        self.client = httpx.Client(
            base_url=api_base,
            headers={"Authorization": f"Bearer {api_key}"},
            timeout=60.0,
        )
        self.model = model

    def _call(self, prompt: str, max_tokens: int = 200) -> str:
        """Make a single LLM call. Returns empty string on failure."""
        try:
            resp = self.client.post(
                "/chat/completions",
                json={
                    "model": self.model,
                    "messages": [{"role": "user", "content": prompt}],
                    "max_tokens": max_tokens,
                    "temperature": 0.0,
                },
            )
            resp.raise_for_status()
            return resp.json()["choices"][0]["message"]["content"].strip()
        except Exception as e:
            logger.warning("LLM judge call failed: %s", e)
            return ""

    def score_response(self, prompt: str, response: str) -> float:
        """Score a response quality (1-10) using LLM judge.

        Falls back to a length-based heuristic on failure.
        """
        judge_prompt = (
            "Rate the quality of the following AI assistant response on a scale of 1-10.\n"
            "Consider: accuracy, completeness, helpfulness, and clarity.\n"
            "Respond with ONLY a single number between 1 and 10.\n\n"
            f"Context/Prompt:\n{prompt[:500]}\n\n"
            f"Response:\n{response[:1000]}\n\n"
            "Score (1-10):"
        )
        result = self._call(judge_prompt, max_tokens=5)
        if result:
            try:
                score = float(result.strip().split()[0])
                return max(1.0, min(10.0, score))
            except (ValueError, IndexError):
                pass

        # Heuristic fallback
        return min(10.0, len(response) / 100 + 3)

    def score_reasoning(self, completion: str) -> float:
        """Score reasoning quality, normalized to [-1, 1].

        Used by GRPO reward function. Falls back to heuristic on failure.
        """
        judge_prompt = (
            "Rate the reasoning quality of the following AI response on a scale of 1-10.\n"
            "Consider: logical structure, step-by-step reasoning, correctness, and clarity.\n"
            "Respond with ONLY a single number between 1 and 10.\n\n"
            f"Response:\n{completion[:1500]}\n\n"
            "Score (1-10):"
        )
        result = self._call(judge_prompt, max_tokens=5)
        if result:
            try:
                raw_score = float(result.strip().split()[0])
                return max(-1.0, min(1.0, (raw_score - 5.5) / 4.5))
            except (ValueError, IndexError):
                pass

        # Heuristic fallback
        return _heuristic_reasoning_score(completion)

    def score_domain(self, prompt: str, generated: str, expected: str) -> dict:
        """Score on accuracy, completeness, faithfulness (1-5 each)."""
        judge_prompt = (
            "You are evaluating an AI assistant's response.\n\n"
            f"Prompt:\n{prompt[:500]}\n\n"
            f"Expected answer:\n{expected[:500]}\n\n"
            f"Generated answer:\n{generated[:500]}\n\n"
            "Rate the generated answer on three dimensions (1-5 each):\n"
            "1. Accuracy: How factually correct is the answer?\n"
            "2. Completeness: Does the answer cover all important points?\n"
            "3. Faithfulness: Is the answer consistent with the expected answer?\n\n"
            'Respond ONLY in JSON: {"accuracy": N, "completeness": N, "faithfulness": N}'
        )
        result = self._call(judge_prompt)
        try:
            return json.loads(result)
        except (json.JSONDecodeError, TypeError):
            return {"accuracy": 3, "completeness": 3, "faithfulness": 3}

    def compare_ab(self, prompt: str, response_a: str, response_b: str) -> str:
        """Blind A/B comparison. Returns 'A', 'B', or 'tie'."""
        judge_prompt = (
            "Compare the two AI responses below. Which one is better?\n\n"
            f"Prompt:\n{prompt[:300]}\n\n"
            f"Response A:\n{response_a[:500]}\n\n"
            f"Response B:\n{response_b[:500]}\n\n"
            "Consider helpfulness, accuracy, and clarity.\n"
            "Respond with ONLY one letter: A, B, or T (for tie)."
        )
        result = self._call(judge_prompt, max_tokens=5)
        result = result.strip().upper()
        if result.startswith("A"):
            return "A"
        elif result.startswith("B"):
            return "B"
        return "tie"

    def check_correctness(self, answer: str, expected: str) -> bool:
        """Check if an answer is correct for open-ended questions."""
        judge_prompt = (
            "Is the following answer correct given the expected answer?\n\n"
            f"Expected: {expected[:300]}\n"
            f"Given: {answer[:300]}\n\n"
            "Respond with ONLY 'yes' or 'no'."
        )
        result = self._call(judge_prompt, max_tokens=5)
        return result.strip().lower().startswith("y")


def _heuristic_reasoning_score(completion: str) -> float:
    """Keyword-based heuristic reward for reasoning quality."""
    score = 0.0
    if len(completion) > 50:
        score += 0.3
    if len(completion) > 200:
        score += 0.2
    reasoning_markers = ["because", "therefore", "however", "first", "then", "finally"]
    for marker in reasoning_markers:
        if marker.lower() in completion.lower():
            score += 0.1
    if len(completion.strip()) < 10:
        score -= 0.5
    return min(1.0, max(-1.0, score))


def create_judge_from_settings(settings=None) -> OpenAICompatibleJudge:
    """Create a judge using the worker's configured LLM settings.

    Accepts an optional settings object for explicit injection.
    Falls back to the global container for backward compatibility.
    """
    if settings is None:
        from src.infra import get_container

        settings = get_container().settings
    return OpenAICompatibleJudge(
        api_base=settings.llm_api_base_url,
        api_key=settings.llm_api_key,
        model=settings.llm_model,
    )
