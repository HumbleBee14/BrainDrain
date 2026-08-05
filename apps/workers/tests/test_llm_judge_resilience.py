"""Tests for the judge's fail-loud resilience policy (no silent fake scores).

The judge must retry transient errors, raise JudgeUnavailableError on
persistent/auth failure by default (so a broken judge fails the run instead of
poisoning rewards/scores), and only fall back to heuristics when explicitly
opted in via on_failure='heuristic'.
"""

import pytest

from src.activities.llm_judge import JudgeUnavailableError, OpenAICompatibleJudge


class _Resp:
    def __init__(self, status_code, content=None, text=""):
        self.status_code = status_code
        self._content = content
        self.text = text

    def json(self):
        return {"choices": [{"message": {"content": self._content}}]}


def _ok(content):
    return _Resp(200, content=content)


def _judge(on_failure="error", max_retries=2):
    return OpenAICompatibleJudge(
        "http://x", "k", "m", max_retries=max_retries, on_failure=on_failure
    )


@pytest.fixture(autouse=True)
def _no_sleep(monkeypatch):
    monkeypatch.setattr("time.sleep", lambda *_a, **_k: None)


def test_success_returns_score(monkeypatch):
    j = _judge()
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _ok("8"))
    assert j.score_response("p", "r") == 8.0


def test_transient_error_retries_then_succeeds(monkeypatch):
    j = _judge(max_retries=3)
    calls = {"n": 0}

    def post(*a, **k):
        calls["n"] += 1
        return _Resp(503) if calls["n"] < 3 else _ok("7")

    monkeypatch.setattr(j.client, "post", post)
    assert j.score_response("p", "r") == 7.0
    assert calls["n"] == 3


def test_persistent_transient_raises(monkeypatch):
    j = _judge(max_retries=1)
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _Resp(503))
    with pytest.raises(JudgeUnavailableError):
        j.score_response("p", "r")


def test_auth_error_raises_without_retry(monkeypatch):
    j = _judge(max_retries=3)
    calls = {"n": 0}

    def post(*a, **k):
        calls["n"] += 1
        return _Resp(401, text="unauthorized")

    monkeypatch.setattr(j.client, "post", post)
    with pytest.raises(JudgeUnavailableError):
        j.score_response("p", "r")
    assert calls["n"] == 1  # non-retryable: no backoff loop


def test_unparseable_score_raises_in_error_mode(monkeypatch):
    j = _judge(max_retries=0)
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _ok("definitely not a number"))
    with pytest.raises(JudgeUnavailableError):
        j.score_response("p", "r")


def test_heuristic_mode_falls_back_instead_of_raising(monkeypatch):
    j = _judge(on_failure="heuristic", max_retries=0)
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _Resp(500))
    val = j.score_response("p", "some response text")
    assert isinstance(val, float)  # heuristic returned, no exception


def test_compare_ab_raises_on_outage_by_default(monkeypatch):
    j = _judge(max_retries=0)
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _Resp(500))
    with pytest.raises(JudgeUnavailableError):
        j.compare_ab("p", "a", "b")


def test_check_correctness_raises_on_outage_by_default(monkeypatch):
    j = _judge(max_retries=0)
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _Resp(500))
    with pytest.raises(JudgeUnavailableError):
        j.check_correctness("answer", "expected")


def test_compare_ab_parses_winner(monkeypatch):
    j = _judge()
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _ok("A"))
    assert j.compare_ab("p", "gold", "sample") == "A"


def test_preflight_names_missing_config_without_calling_api(monkeypatch):
    j = OpenAICompatibleJudge("http://x", "", "m", max_retries=0)
    monkeypatch.setattr(
        j.client, "post", lambda *a, **k: pytest.fail("preflight must not call a keyless API")
    )
    with pytest.raises(JudgeUnavailableError, match="API key"):
        j.preflight()


def test_preflight_rejects_unreachable_judge(monkeypatch):
    j = _judge(max_retries=0)
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _Resp(401, text="bad key"))
    with pytest.raises(JudgeUnavailableError, match="401"):
        j.preflight()


def test_preflight_passes_when_judge_answers(monkeypatch):
    j = _judge(max_retries=0)
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _ok("OK"))
    j.preflight()


def test_failure_message_keeps_root_cause(monkeypatch):
    j = _judge(max_retries=0)
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _Resp(401, text="invalid api key"))
    with pytest.raises(JudgeUnavailableError, match="invalid api key"):
        j.score_domain("p", "g", "e")


def test_verdict_calls_send_the_configured_completion_budget(monkeypatch):
    """A reasoning judge spends its budget inside <think> before the verdict, so
    every scoring call must send the configured budget, not a tiny literal."""
    j = OpenAICompatibleJudge("http://x", "k", "m", max_completion_tokens=4096)
    sent = {}

    def _post(_path, json):
        sent["max_tokens"] = json["max_tokens"]
        return _ok("<think>hmm</think>8")

    monkeypatch.setattr(j.client, "post", _post)
    assert j.score_response("p", "r") == 8.0
    assert sent["max_tokens"] == 4096


def test_reasoning_block_is_stripped_from_verdict(monkeypatch):
    j = _judge()
    monkeypatch.setattr(j.client, "post", lambda *a, **k: _ok("<think>x</think>\nyes"))
    assert j.check_correctness("a", "e") is True


def test_client_timeout_is_configurable():
    j = OpenAICompatibleJudge("http://x", "k", "m", timeout_seconds=300.0)
    assert j.client.timeout.read == 300.0


def test_thinking_disabled_appends_soft_switch(monkeypatch):
    j = OpenAICompatibleJudge("http://x", "k", "m")
    seen = {}

    def _post(_path, json):
        seen["prompt"] = json["messages"][0]["content"]
        return _ok("8")

    monkeypatch.setattr(j.client, "post", _post)
    j.score_response("p", "r")
    assert seen["prompt"].endswith("/no_think")


def test_thinking_enabled_leaves_prompt_untouched(monkeypatch):
    j = OpenAICompatibleJudge("http://x", "k", "m", enable_thinking=True)
    seen = {}

    def _post(_path, json):
        seen["prompt"] = json["messages"][0]["content"]
        return _ok("8")

    monkeypatch.setattr(j.client, "post", _post)
    j.score_response("p", "r")
    assert "/no_think" not in seen["prompt"]
