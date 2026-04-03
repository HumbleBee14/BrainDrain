"""Centralized status constants and shared values for database operations.

These mirror the Rust definitions in crates/shared/src/.
Keep in sync with the Rust source of truth.
"""


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


class EvaluationStatus:
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"


class DeploymentStatus:
    UNDEPLOYED = "undeployed"
    DEPLOYING = "deploying"
    ACTIVE = "active"
    INACTIVE = "inactive"


# GPU hourly rates by class — mirrors crates/shared/src/constants.rs GPU_HOURLY_RATES.
# If you change these, update the Rust source too.
GPU_HOURLY_RATES: dict[str, float] = {
    "t4": 0.80,
    "a10g": 1.20,
    "l40s": 1.80,
    "a10040gb": 2.00,
    "a10080gb": 3.00,
    "h100": 4.50,
}
GPU_DEFAULT_HOURLY_RATE: float = 0.80
