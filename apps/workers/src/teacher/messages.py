"""User-facing copy shared by every teacher code path.

These sentences reach a person, so they must read identically wherever they are
raised — a user who hits the same wall twice should not be told two different
things. The API states the same rules in Rust
(`crates/api/src/services/teacher/fidelity.rs`); `test_teacher_messages.py`
asserts the two languages have not drifted apart.
"""

TOKENIZER_MISMATCH_MESSAGE = (
    "These two models read text differently, so high-fidelity training is not "
    "possible between them. Standard distillation works — switch and re-run."
)

SPEND_CAP_MESSAGE = (
    "This run reached your GPU spending cap for teachers. Raise the cap in "
    "Settings → Billing or resume with a smaller dataset."
)
