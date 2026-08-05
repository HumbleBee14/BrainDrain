"""Unified LLM-as-Judge module used by both training and evaluation.

Provides a single JudgeLLM class that handles:
  - Domain scoring (accuracy, completeness, faithfulness)
  - A/B comparison (blind pairwise)
  - Correctness checking (for benchmarks)
  - Response quality scoring (for DPO pair creation)
  - Reasoning reward scoring (for GRPO)
  - Refusal classification (for safety checks)

All judge calls go through the same OpenAI-compatible API client. Transient
errors are retried with backoff; a persistently-unavailable judge raises
JudgeUnavailableError (default) rather than silently returning a fabricated
score — see the on_failure policy on OpenAICompatibleJudge.
"""

import json
import logging
import random
import time
from typing import Protocol

import httpx

from src.failure_message import NO_LLM_KEY
from src.llm_output import answer_text

logger = logging.getLogger("platform.judge")

# HTTP statuses worth retrying (transient): rate limit + gateway/5xx.
_RETRYABLE_STATUS = {429, 500, 502, 503, 504}


class JudgeUnavailableError(RuntimeError):
    """Raised when the judge LLM cannot produce a usable result.

    This propagates out of scoring calls (unless on_failure='heuristic') so a
    broken judge fails the training/eval activity loudly instead of silently
    poisoning GRPO rewards, DPO pair selection, or eval scores with fabricated
    heuristic numbers. See the repo's "Correctness Over Convenience" rules.
    """


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

    def preflight(self) -> None:
        """Raise JudgeUnavailableError unless configured and reachable."""
        ...


class OpenAICompatibleJudge:
    """LLM-as-Judge backed by any OpenAI-compatible API.

    Handles all judge operations with consistent error handling and
    heuristic fallbacks when the API is unavailable.
    """

    def __init__(
        self,
        api_base: str,
        api_key: str,
        model: str,
        max_retries: int = 3,
        on_failure: str = "error",
        max_completion_tokens: int = 2000,
        timeout_seconds: float = 600.0,
        enable_thinking: bool = False,
    ):
        # Reasoning judges think for tens of seconds per verdict, and a
        # scale-to-zero judge endpoint can take minutes to cold-start.
        self.client = httpx.Client(
            base_url=api_base,
            headers={"Authorization": f"Bearer {api_key}"},
            timeout=timeout_seconds,
        )
        self.api_base = api_base
        self.api_key = api_key
        self.model = model
        self.max_retries = max_retries
        # Verdicts are a handful of tokens, but reasoning judges spend their
        # budget inside <think> before emitting them — the budget must cover
        # the thinking, and answer_text() recovers the short verdict after.
        self.max_completion_tokens = max_completion_tokens
        # Off by default: a reasoning judge deliberates 30-60s per verdict,
        # which turns every judge-bound stage (eval, DPO pair filtering, GRPO
        # rewards) from minutes into hours. The soft switch below is plain
        # prompt text: reasoning models that support it skip the think block;
        # everything else ignores a trailing token.
        self.enable_thinking = enable_thinking
        # "error"  → raise JudgeUnavailableError so the run fails loudly (default,
        #            correctness-first). "heuristic" → advanced opt-in: log and
        #            fall back to the length/keyword heuristics.
        self.on_failure = on_failure

    def _call(self, prompt: str, max_tokens: int | None = None) -> str:
        """Call the judge, retrying transient errors with backoff+jitter.

        Raises JudgeUnavailableError if no usable response is obtained after
        max_retries+1 attempts, or immediately on a non-retryable HTTP error
        (e.g. 401/403 bad credentials, 400 bad request). Never returns "" —
        callers must not mistake an outage for a low-quality response.
        """
        if not self.enable_thinking:
            prompt = f"{prompt} /no_think"
        last_err: str | None = None
        for attempt in range(self.max_retries + 1):
            try:
                resp = self.client.post(
                    "/chat/completions",
                    json={
                        "model": self.model,
                        "messages": [{"role": "user", "content": prompt}],
                        "max_tokens": max_tokens or self.max_completion_tokens,
                        "temperature": 0.0,
                    },
                )
            except (httpx.TimeoutException, httpx.TransportError) as e:
                last_err = f"{type(e).__name__}: {e}"
            else:
                if resp.status_code in _RETRYABLE_STATUS:
                    last_err = f"HTTP {resp.status_code}"
                elif resp.status_code >= 400:
                    # Non-retryable (auth / bad request): retrying won't help.
                    raise JudgeUnavailableError(f"judge HTTP {resp.status_code}: {resp.text[:200]}")
                else:
                    try:
                        content = resp.json()["choices"][0]["message"]["content"]
                        return answer_text(content).strip()
                    except (KeyError, IndexError, ValueError, TypeError) as e:
                        last_err = f"malformed response: {e}"

            if attempt < self.max_retries:
                # Exponential backoff (cap 8s) + full jitter to avoid thundering herd.
                time.sleep(min(2**attempt, 8) + random.uniform(0, 0.5))

        raise JudgeUnavailableError(
            f"judge unavailable after {self.max_retries + 1} attempts: {last_err}"
        )

    def preflight(self) -> None:
        """Check the judge is configured and reachable before expensive work.

        Evaluation spends GPU minutes generating answers before the first judge
        call, so an unusable judge must surface here rather than at scoring time.
        """
        missing = [
            name
            for name, value in (
                ("endpoint URL", self.api_base),
                ("API key", self.api_key),
                ("model", self.model),
            )
            if not (value or "").strip()
        ]
        if missing:
            raise JudgeUnavailableError(f"{NO_LLM_KEY} (missing {', '.join(missing)})")
        self._call("Reply with OK.", max_tokens=1)

    def _handle_failure(self, what: str, heuristic, cause: Exception | None = None):
        """Apply the on_failure policy: raise (default) or log + heuristic."""
        if self.on_failure == "heuristic":
            logger.warning(
                "Judge %s failed; using heuristic (on_failure='heuristic'): %s", what, cause
            )
            return heuristic()
        # Keep the cause in the message: it names the actual fault (bad key,
        # unreachable endpoint, unparseable reply) that the operator must fix.
        detail = f": {cause}" if cause else ""
        raise JudgeUnavailableError(f"judge {what} produced no usable result{detail}") from cause

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

        def _heuristic():
            return min(10.0, len(response) / 100 + 3)

        try:
            result = self._call(judge_prompt)
            return max(1.0, min(10.0, float(result.strip().split()[0])))
        except (ValueError, IndexError) as e:
            return self._handle_failure("score_response (unparseable)", _heuristic, e)
        except JudgeUnavailableError as e:
            return self._handle_failure("score_response", _heuristic, e)

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
        try:
            result = self._call(judge_prompt)
            raw_score = float(result.strip().split()[0])
            return max(-1.0, min(1.0, (raw_score - 5.5) / 4.5))
        except (ValueError, IndexError) as e:
            return self._handle_failure(
                "score_reasoning (unparseable)", lambda: _heuristic_reasoning_score(completion), e
            )
        except JudgeUnavailableError as e:
            return self._handle_failure(
                "score_reasoning", lambda: _heuristic_reasoning_score(completion), e
            )

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

        # On failure return no dimensions so the caller excludes the sample
        # instead of scoring a fabricated midpoint.
        def _heuristic():
            return {}

        try:
            return json.loads(self._call(judge_prompt))
        except (json.JSONDecodeError, TypeError) as e:
            return self._handle_failure("score_domain (unparseable)", _heuristic, e)
        except JudgeUnavailableError as e:
            return self._handle_failure("score_domain", _heuristic, e)

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
        try:
            result = self._call(judge_prompt).strip().upper()
        except JudgeUnavailableError as e:
            return self._handle_failure("compare_ab", lambda: "tie", e)
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
        try:
            result = self._call(judge_prompt)
        except JudgeUnavailableError as e:
            return self._handle_failure("check_correctness", lambda: False, e)
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


async def create_judge_for_tenant(
    db,
    tenant_id: str,
    settings=None,
) -> OpenAICompatibleJudge:
    """Create a judge using tenant-specific LLM config from the database.

    Falls back to worker-level env var defaults if the tenant has no custom config.
    """
    if settings is None:
        from src.infra import get_container

        settings = get_container().settings

    from src.tenant_config import get_tenant_llm_config

    llm_config = await get_tenant_llm_config(
        db=db,
        tenant_id=tenant_id,
        default_api_base_url=settings.llm_api_base_url,
        default_api_key=settings.llm_api_key,
        default_model=settings.llm_model,
        encryption_key=getattr(settings, "settings_encryption_key", None),
    )

    return OpenAICompatibleJudge(
        api_base=llm_config.api_base_url,
        api_key=llm_config.api_key,
        model=llm_config.model,
        max_retries=getattr(settings, "judge_max_retries", 3),
        on_failure=getattr(settings, "judge_on_failure", "error"),
        max_completion_tokens=llm_config.max_tokens,
        timeout_seconds=getattr(settings, "judge_timeout_seconds", 600.0),
        enable_thinking=getattr(settings, "judge_enable_thinking", False),
    )
