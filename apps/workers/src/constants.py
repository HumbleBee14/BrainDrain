"""Centralized status constants and shared values for database operations.

Auto-generated from Rust source of truth. Do not edit manually.
Run: python scripts/sync_constants.py
"""

# ── AUTO-GENERATED FROM crates/shared/src/constants.rs ──
# DO NOT EDIT MANUALLY — run: python scripts/sync_constants.py

GPU_HOURLY_RATES: dict[str, float] = {
    "t4": 0.8,
    "a10g": 1.2,
    "l40s": 1.8,
    "a10040gb": 2.0,
    "a10080gb": 3.0,
    "h100": 4.5,
    "a10080gb_dual": 6.0,
    "h100_dual": 9.0,
}
GPU_DEFAULT_HOURLY_RATE: float = 0.8

# ── END AUTO-GENERATED ──

# Maps the platform's gpu_class values to current Modal GPU type strings
# (Modal 1.0+ uses string specifiers; the old modal.gpu.* object API is deprecated).
# Keyed by the canonical GpuClass value (lowercase) the API bills against, so
# the provisioned Modal hardware matches the charged rate.
# Distillation methods, mirroring the Rust `DistillMethod` enum. Defined once here
# because four layers key off them — the workflow that admits a plan, the strategy
# registry, the trainer config, and the provider that picks which image runs the
# job — and a typo in any one of them would route a run to the wrong path.
TEXT_DISTILL_METHOD = "text"
LOGIT_DISTILL_METHOD = "logit"
ON_POLICY_DISTILL_METHOD = "on_policy"

MODAL_GPU_MAP: dict[str, str] = {
    "t4": "T4",
    "a10g": "A10G",
    "l40s": "L40S",
    "a10040gb": "A100",
    "a10080gb": "A100-80GB",
    "h100": "H100",
    # Multi-device classes serve on-policy distillation: the teacher needs a card
    # of its own beside the trainer. Kept in sync with GpuClass by
    # tests/test_constants.py::test_every_rust_gpu_class_maps_to_a_modal_gpu — an
    # unmapped class falls back to a single MODAL_DEFAULT_GPU, which would put
    # teacher and student on one small card instead of failing.
    "a10080gb_dual": "A100-80GB:2",
    "h100_dual": "H100:2",
}
MODAL_DEFAULT_GPU: str = "T4"

# Maps gpu_class values to Beam GPU type strings (beam.GpuType values), for
# gpu_provider="beam". Unlike the Modal map there is NO default fallback: an
# unmapped class raises in BeamGpuProvider, because Beam schedules whatever
# type string it is handed and a silent substitute would bill one class while
# running another. Multi-device classes are deliberately absent until Beam
# multi-GPU task queues are validated end to end.
BEAM_GPU_MAP: dict[str, str] = {
    "t4": "T4",
    "a10g": "A10G",
    "l40s": "L40S",
    "a10040gb": "A100-40",
    "a10080gb": "A100-80",
    "h100": "H100",
}

# Devices per GPU class, mirroring `GpuClass::device_count()`. Billing needs this
# separately from the Modal map because it applies to the local provider too: an
# on-policy container's cost has to be split between the student's training time
# and the teacher's serving time, and the device count is what that split is by.
# Kept honest by tests/test_constants.py against both the Rust rate table and the
# Modal map.
GPU_DEVICE_COUNTS: dict[str, int] = {
    "a10080gb_dual": 2,
    "h100_dual": 2,
}
GPU_DEFAULT_DEVICE_COUNT: int = 1


# ── AUTO-GENERATED FROM crates/shared/src/enums.rs ──
# DO NOT EDIT MANUALLY — run: python scripts/sync_constants.py


class DocumentStatus:
    UPLOADED = "uploaded"
    PARSING = "parsing"
    PARSED = "parsed"
    FAILED = "failed"


class DatasetStatus:
    GENERATING = "generating"
    REVIEW_PENDING = "review_pending"
    APPROVED = "approved"
    ARCHIVED = "archived"
    FAILED = "failed"


class TrainingJobStatus:
    PENDING = "pending"
    COST_APPROVAL = "cost_approval"
    PROVISIONING = "provisioning"
    TRAINING = "training"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"


class DeploymentStatus:
    UNDEPLOYED = "undeployed"
    DEPLOYING = "deploying"
    ACTIVE = "active"
    INACTIVE = "inactive"


class EvaluationStatus:
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"


# ── END AUTO-GENERATED ENUMS ──
