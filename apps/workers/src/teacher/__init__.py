"""Teacher-model access for distillation.

The teacher is an external LLM chosen per run — its own endpoint, model and
key, distinct from the tenant's configured LLM. Everything that touches
teacher credentials or provenance goes through this package:

- `TeacherClient` is the only place a teacher API key is decrypted, and it
  guarantees the URL guard runs before any request.
- `provenance` is the only reader/writer of the `datasets.config.teacher`
  block, so its shape stays consistent across activities.
- `tokenizer_identity` is the only place that proves a teacher and student
  tokenizer are byte-identical, a precondition for offline logit/KL
  distillation (Stage 2).
- `rendering` is the only place the prompt/completion token boundary is
  computed, so teacher scoring and student training cannot disagree about which
  positions are supervised.
"""

from src.teacher.client import TeacherClient, TeacherConfig, parse_teacher_config
from src.teacher.provenance import build_provenance, read_provenance, teacher_host
from src.teacher.rendering import (
    RenderedRecord,
    RenderingError,
    TokenCounts,
    count_tokens,
    render_dataset,
    render_record,
    rendering_fingerprint,
)
from src.teacher.tokenizer_identity import (
    TokenizerArtifactFetchError,
    TokenizerIdentityResult,
    check_tokenizer_identity,
)

__all__ = [
    "RenderedRecord",
    "RenderingError",
    "TeacherClient",
    "TeacherConfig",
    "TokenCounts",
    "TokenizerArtifactFetchError",
    "TokenizerIdentityResult",
    "build_provenance",
    "check_tokenizer_identity",
    "count_tokens",
    "parse_teacher_config",
    "read_provenance",
    "render_dataset",
    "render_record",
    "rendering_fingerprint",
    "teacher_host",
]
