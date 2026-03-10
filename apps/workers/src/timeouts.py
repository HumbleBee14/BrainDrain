"""Centralized activity timeout values, driven by WorkerSettings.

All workflow files import from here instead of hardcoding timedelta values.
Override any timeout via the corresponding APP_TIMEOUT_* environment variable.

Example:
    APP_TIMEOUT_TRAIN_HOURS=12  # allow 12h for very large model training
"""

from datetime import timedelta

from src.config import WorkerSettings


def _s() -> WorkerSettings:
    """Lazy singleton — avoids importing at module level before settings are loaded."""
    from src.infra import get_container

    return get_container().settings


# Parse
def parse_activity() -> timedelta:
    return timedelta(minutes=_s().timeout_parse_minutes)


def parse_heartbeat() -> timedelta:
    return timedelta(minutes=max(2, _s().timeout_parse_minutes // 5))


# Chunk
def chunk_activity() -> timedelta:
    return timedelta(minutes=_s().timeout_chunk_minutes)


# Generate pairs
def generate_pairs_activity() -> timedelta:
    return timedelta(minutes=_s().timeout_generate_pairs_minutes)


def generate_pairs_heartbeat() -> timedelta:
    return timedelta(minutes=max(5, _s().timeout_generate_pairs_minutes // 6))


# Build dataset
def build_dataset_activity() -> timedelta:
    return timedelta(minutes=_s().timeout_build_dataset_minutes)


# Training
def train_activity() -> timedelta:
    return timedelta(hours=_s().timeout_train_hours)


def train_heartbeat() -> timedelta:
    return timedelta(minutes=5)


# Iterative training (one round)
def train_iterative_activity() -> timedelta:
    return timedelta(hours=_s().timeout_train_iterative_hours)


# Holdout evaluation during training
def holdout_eval_activity() -> timedelta:
    return timedelta(hours=_s().timeout_holdout_eval_hours)


def holdout_eval_heartbeat() -> timedelta:
    return timedelta(minutes=5)


# Full evaluation suite
def eval_activity() -> timedelta:
    return timedelta(hours=_s().timeout_eval_hours)


def eval_heartbeat() -> timedelta:
    return timedelta(minutes=10)


# GGUF export
def export_activity() -> timedelta:
    return timedelta(hours=_s().timeout_export_hours)


def export_heartbeat() -> timedelta:
    return timedelta(minutes=15)


# Lightweight lookups
def db_lookup() -> timedelta:
    return timedelta(seconds=30)
