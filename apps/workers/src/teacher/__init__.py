"""Teacher-model access for distillation.

The teacher is an external LLM chosen per run — its own endpoint, model and
key, distinct from the tenant's configured LLM. Everything that touches
teacher credentials or provenance goes through this package:

- `TeacherClient` is the only place a teacher API key is decrypted, and it
  guarantees the URL guard runs before any request.
- `provenance` is the only reader/writer of the `datasets.config.teacher`
  block, so its shape stays consistent across activities.
"""

from src.teacher.client import TeacherClient, TeacherConfig, parse_teacher_config
from src.teacher.provenance import build_provenance, read_provenance, teacher_host

__all__ = [
    "TeacherClient",
    "TeacherConfig",
    "build_provenance",
    "parse_teacher_config",
    "read_provenance",
    "teacher_host",
]
