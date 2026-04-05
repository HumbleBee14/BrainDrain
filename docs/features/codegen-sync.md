# Rust/Python Codegen Sync

**PR:** #23  
**Problem:** GPU hourly rates and status enums (like `TrainingJobStatus`)
exist in both Rust and Python. When someone changes a rate or adds a status
variant in Rust, the Python side doesn't update automatically. The two
codebases drift apart, and you get bugs like: Python writes "provisioning"
to the DB but Rust only knows about "pending".

## How it works

A single script reads the Rust source of truth and generates Python code:

```
crates/shared/src/constants.rs  ──→  scripts/sync_constants.py  ──→  apps/workers/src/constants.py
crates/shared/src/enums.rs      ──┘
```

The generated Python file has marker comments (`AUTO-GENERATED`) so the
script knows which sections to replace on each run.

## What's synced

| From Rust | Generated in Python |
|---|---|
| `GPU_HOURLY_RATES` | `GPU_HOURLY_RATES` dict |
| `GPU_DEFAULT_HOURLY_RATE` | `GPU_DEFAULT_HOURLY_RATE` float |
| `DocumentStatus` enum | `DocumentStatus` class with string constants |
| `DatasetStatus` enum | `DatasetStatus` class |
| `TrainingJobStatus` enum | `TrainingJobStatus` class |
| `DeploymentStatus` enum | `DeploymentStatus` class |
| `EvaluationStatus` enum | `EvaluationStatus` class |

## Drift detection

CI runs `python scripts/sync_constants.py --check` on every PR. If Rust and
Python are out of sync, the build fails with:

```
DRIFT DETECTED: Python constants/enums are out of sync with Rust.
Run: python scripts/sync_constants.py
```

The pre-commit hook also catches this before you can commit.

## Files

- `scripts/sync_constants.py` — Generator script
- `crates/shared/src/constants.rs` — Rust source (GPU rates)
- `crates/shared/src/enums.rs` — Rust source (status enums)
- `apps/workers/src/constants.py` — Generated Python output
