"""Tokenizer identity guard: canonicalization, mismatch detection, combined hash.

No network — every fetcher here reads fixture files written under `tmp_path`,
mirroring what `hf_hub_download` would hand back (a local file path per
model/filename), but fully offline.
"""

from pathlib import Path

import pytest

from src.teacher.tokenizer_identity import (
    ArtifactFetcher,
    check_tokenizer_identity,
    clear_cache,
    compute_tokenizer_hashes,
)


@pytest.fixture(autouse=True)
def _reset_cache():
    clear_cache()
    yield
    clear_cache()


def _write_model(root: Path, model_id: str, files: dict[str, str]) -> None:
    model_dir = root / model_id
    model_dir.mkdir(parents=True, exist_ok=True)
    for filename, content in files.items():
        (model_dir / filename).write_text(content, encoding="utf-8")


def _fetcher_over(root: Path) -> ArtifactFetcher:
    def fetch(model_id: str, filename: str, revision: str | None, hf_token: str) -> bytes | None:
        path = root / model_id / filename
        if not path.exists():
            return None
        return path.read_bytes()

    return fetch


TOKENIZER_JSON = '{"model": {"vocab": {"hello": 0, "world": 1}}, "added_tokens": []}'
SPECIAL_TOKENS_MAP = '{"bos_token": "<s>", "eos_token": "</s>"}'
CHAT_TEMPLATE = "{% for m in messages %}{{ m.role }}: {{ m.content }}\n{% endfor %}"


def test_identical_artifacts_are_compatible(tmp_path):
    files = {
        "tokenizer.json": TOKENIZER_JSON,
        "special_tokens_map.json": SPECIAL_TOKENS_MAP,
        "chat_template.jinja": CHAT_TEMPLATE,
    }
    _write_model(tmp_path, "teacher", files)
    _write_model(tmp_path, "student", files)

    result = check_tokenizer_identity("teacher", "student", fetcher=_fetcher_over(tmp_path))

    assert result.compatible
    assert result.mismatched_artifacts == ()
    assert result.teacher.combined_hash == result.student.combined_hash


def test_differing_special_tokens_map_is_incompatible_and_named(tmp_path):
    _write_model(
        tmp_path,
        "teacher",
        {"tokenizer.json": TOKENIZER_JSON, "special_tokens_map.json": SPECIAL_TOKENS_MAP},
    )
    _write_model(
        tmp_path,
        "student",
        {
            "tokenizer.json": TOKENIZER_JSON,
            "special_tokens_map.json": '{"bos_token": "<s>", "eos_token": "<|end|>"}',
        },
    )

    result = check_tokenizer_identity("teacher", "student", fetcher=_fetcher_over(tmp_path))

    assert not result.compatible
    assert result.mismatched_artifacts == ("special_tokens_map",)


def test_differing_chat_template_is_incompatible(tmp_path):
    _write_model(
        tmp_path,
        "teacher",
        {"tokenizer.json": TOKENIZER_JSON, "chat_template.jinja": CHAT_TEMPLATE},
    )
    _write_model(
        tmp_path,
        "student",
        {
            "tokenizer.json": TOKENIZER_JSON,
            "chat_template.jinja": "{% for m in messages %}{{ m.content }}{% endfor %}",
        },
    )

    result = check_tokenizer_identity("teacher", "student", fetcher=_fetcher_over(tmp_path))

    assert not result.compatible
    assert result.mismatched_artifacts == ("chat_template",)


def test_semantically_identical_json_with_different_formatting_is_compatible(tmp_path):
    compact = '{"bos_token":"<s>","eos_token":"</s>","pad_token":"<pad>"}'
    reformatted = '{\n  "pad_token": "<pad>",\n  "eos_token": "</s>",\n  "bos_token": "<s>"\n}\n'
    _write_model(
        tmp_path,
        "teacher",
        {"tokenizer.json": TOKENIZER_JSON, "special_tokens_map.json": compact},
    )
    _write_model(
        tmp_path,
        "student",
        {"tokenizer.json": TOKENIZER_JSON, "special_tokens_map.json": reformatted},
    )

    result = check_tokenizer_identity("teacher", "student", fetcher=_fetcher_over(tmp_path))

    assert result.compatible
    assert result.mismatched_artifacts == ()


def test_optional_artifact_missing_on_both_sides_is_compatible(tmp_path):
    _write_model(tmp_path, "teacher", {"tokenizer.json": TOKENIZER_JSON})
    _write_model(tmp_path, "student", {"tokenizer.json": TOKENIZER_JSON})

    result = check_tokenizer_identity("teacher", "student", fetcher=_fetcher_over(tmp_path))

    assert result.compatible
    assert result.teacher.hash_for("special_tokens_map") is None
    assert result.student.hash_for("special_tokens_map") is None


def test_optional_artifact_missing_on_only_one_side_is_incompatible(tmp_path):
    _write_model(
        tmp_path,
        "teacher",
        {"tokenizer.json": TOKENIZER_JSON, "special_tokens_map.json": SPECIAL_TOKENS_MAP},
    )
    _write_model(tmp_path, "student", {"tokenizer.json": TOKENIZER_JSON})

    result = check_tokenizer_identity("teacher", "student", fetcher=_fetcher_over(tmp_path))

    assert not result.compatible
    assert result.mismatched_artifacts == ("special_tokens_map",)


def test_combined_hash_stable_across_calls(tmp_path):
    _write_model(
        tmp_path,
        "solo",
        {"tokenizer.json": TOKENIZER_JSON, "special_tokens_map.json": SPECIAL_TOKENS_MAP},
    )
    fetcher = _fetcher_over(tmp_path)

    first = compute_tokenizer_hashes("solo", fetcher=fetcher)
    clear_cache()
    second = compute_tokenizer_hashes("solo", fetcher=fetcher)

    assert first.combined_hash == second.combined_hash


def test_combined_hash_differs_when_any_artifact_differs(tmp_path):
    _write_model(
        tmp_path,
        "base",
        {"tokenizer.json": TOKENIZER_JSON, "special_tokens_map.json": SPECIAL_TOKENS_MAP},
    )
    _write_model(
        tmp_path,
        "changed_vocab",
        {
            "tokenizer.json": '{"model": {"vocab": {"hello": 0, "world": 2}}, "added_tokens": []}',
            "special_tokens_map.json": SPECIAL_TOKENS_MAP,
        },
    )
    _write_model(
        tmp_path,
        "changed_special_tokens",
        {
            "tokenizer.json": TOKENIZER_JSON,
            "special_tokens_map.json": '{"bos_token": "<s>", "eos_token": "<|end|>"}',
        },
    )
    fetcher = _fetcher_over(tmp_path)

    base = compute_tokenizer_hashes("base", fetcher=fetcher)
    changed_vocab = compute_tokenizer_hashes("changed_vocab", fetcher=fetcher)
    changed_special = compute_tokenizer_hashes("changed_special_tokens", fetcher=fetcher)

    assert base.combined_hash != changed_vocab.combined_hash
    assert base.combined_hash != changed_special.combined_hash
    assert changed_vocab.combined_hash != changed_special.combined_hash


def test_compute_tokenizer_hashes_caches_per_model_id(tmp_path):
    _write_model(tmp_path, "cached", {"tokenizer.json": TOKENIZER_JSON})
    calls: list[str] = []

    def counting_fetcher(model_id, filename, revision, hf_token):
        calls.append(filename)
        path = tmp_path / model_id / filename
        return path.read_bytes() if path.exists() else None

    compute_tokenizer_hashes("cached", fetcher=counting_fetcher)
    calls_after_first = len(calls)
    compute_tokenizer_hashes("cached", fetcher=counting_fetcher)

    assert len(calls) == calls_after_first


def test_vocab_falls_back_to_vocab_and_merges_when_no_tokenizer_json(tmp_path):
    files = {"vocab.json": '{"hello": 0, "world": 1}', "merges.txt": "h e\nw o\n"}
    _write_model(tmp_path, "teacher", files)
    _write_model(tmp_path, "student", files)

    result = check_tokenizer_identity("teacher", "student", fetcher=_fetcher_over(tmp_path))

    assert result.compatible
    assert result.teacher.hash_for("vocab") is not None


def test_merges_txt_trailing_whitespace_and_crlf_are_normalized(tmp_path):
    _write_model(
        tmp_path,
        "teacher",
        {"vocab.json": '{"a": 0}', "merges.txt": "h e  \nw o\n"},
    )
    _write_model(
        tmp_path,
        "student",
        {"vocab.json": '{"a": 0}', "merges.txt": "h e\r\nw o\r\n"},
    )

    result = check_tokenizer_identity("teacher", "student", fetcher=_fetcher_over(tmp_path))

    assert result.compatible


def test_added_tokens_embedded_in_tokenizer_json_is_compared(tmp_path):
    teacher_tokenizer = (
        '{"model": {"vocab": {"hello": 0}}, "added_tokens": [{"id": 100, "content": "<extra>"}]}'
    )
    student_tokenizer = '{"model": {"vocab": {"hello": 0}}, "added_tokens": []}'
    _write_model(tmp_path, "teacher", {"tokenizer.json": teacher_tokenizer})
    _write_model(tmp_path, "student", {"tokenizer.json": student_tokenizer})

    result = check_tokenizer_identity("teacher", "student", fetcher=_fetcher_over(tmp_path))

    assert not result.compatible
    assert "added_tokens" in result.mismatched_artifacts


def test_chat_template_falls_back_to_tokenizer_config_field(tmp_path):
    config = '{"model_max_length": 4096, "chat_template": "hi {{ x }}"}'
    _write_model(tmp_path, "teacher", {"tokenizer_config.json": config})
    _write_model(tmp_path, "student", {"tokenizer_config.json": config})

    result = check_tokenizer_identity("teacher", "student", fetcher=_fetcher_over(tmp_path))

    assert result.compatible
    assert result.teacher.hash_for("chat_template") is not None
