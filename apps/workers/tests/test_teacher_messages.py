"""User-facing teacher copy must not drift between the API and the workers.

The same wall can be hit from either side — the API refuses an ineligible pair up
front, the extraction job refuses it again at run time — and a user told two
different things about one problem reasonably concludes there are two problems.
These tests read the Rust source so a change on either side fails here rather
than reaching a person.
"""

import re
from pathlib import Path

from src.teacher.messages import SPEND_CAP_MESSAGE, TOKENIZER_MISMATCH_MESSAGE

REPO_ROOT = Path(__file__).resolve().parents[3]
FIDELITY_RS = REPO_ROOT / "crates/api/src/services/teacher/fidelity.rs"
BILLING_RS = REPO_ROOT / "crates/api/src/services/teacher/billing.rs"


def rust_strings(path: Path) -> str:
    """Rust source with line-continuation escapes collapsed.

    Multi-line Rust string literals join with `\\` plus leading whitespace, so the
    raw file never contains the sentence a user sees.
    """
    return re.sub(r"\\\s*\n\s*", "", path.read_text(encoding="utf-8"))


def test_tokenizer_mismatch_copy_matches_the_api():
    assert FIDELITY_RS.exists(), f"expected the API's fidelity module at {FIDELITY_RS}"
    assert TOKENIZER_MISMATCH_MESSAGE in rust_strings(FIDELITY_RS)


def test_spend_cap_copy_matches_the_api():
    assert BILLING_RS.exists(), f"expected the API's teacher billing module at {BILLING_RS}"
    assert SPEND_CAP_MESSAGE in rust_strings(BILLING_RS)


def test_the_workers_define_this_copy_exactly_once():
    """A second literal copy is how the two sides drift in the first place."""
    sentence = "These two models read text differently"
    offenders = [
        path.relative_to(REPO_ROOT).as_posix()
        for path in (REPO_ROOT / "apps/workers/src").rglob("*.py")
        if path.name != "messages.py" and sentence in path.read_text(encoding="utf-8")
    ]

    assert offenders == [], f"copy duplicated outside messages.py: {offenders}"
