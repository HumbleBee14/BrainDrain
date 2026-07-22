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
}
GPU_DEFAULT_HOURLY_RATE: float = 0.8

# ── END AUTO-GENERATED ──

# Maps the platform's gpu_class values to current Modal GPU type strings
# (Modal 1.0+ uses string specifiers; the old modal.gpu.* object API is deprecated).
MODAL_GPU_MAP: dict[str, str] = {
    "A10G": "A10",
    "A10": "A10",
    "A100": "A100",
    "A100-80GB": "A100-80GB",
    "H100": "H100",
    "L4": "L4",
    "T4": "T4",
}
MODAL_DEFAULT_GPU: str = "A10"


# ── AUTO-GENERATED FROM crates/shared/src/enums.rs ──
# DO NOT EDIT MANUALLY — run: python scripts/sync_constants.py


class DocumentStatus:
    UPLOADED = "uploaded"
    SCANNING = "scanning"
    PARSING = "parsing"
    PARSED = "parsed"
    FAILED = "failed"


class DatasetStatus:
    GENERATING = "generating"
    REVIEW_PENDING = "review_pending"
    APPROVED = "approved"
    ARCHIVED = "archived"


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
