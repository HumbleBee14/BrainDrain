# crates/shared

**Shared Rust types, enums, and constants used across all backend crates.**

| | |
|---|---|
| Language | Rust |
| Type | Library crate |
| Depends on | Nothing (leaf dependency) |
| Used by | `crates/api`, `crates/db`, `crates/storage` |

## What's Inside

| File | Purpose |
|---|---|
| `enums.rs` | 13 status/type enums (`DocumentStatus`, `TrainingJobStatus`, `PipelineStage`, etc.) — all derive `Serialize`/`Deserialize`/`Display`/`EnumString` |
| `constants.rs` | Temporal queue names, Redis key prefixes, upload size limits (500MB), supported file extensions |
| `s3_paths.rs` | Tenant-scoped S3 path builders (`upload_path`, `dataset_path`, `adapter_file`, etc.) with unit tests |
| `events.rs` | Pipeline event structs for the message bus (`DocumentUploadedEvent`, `TrainingCompletedEvent`, etc.) |

## Usage

This is a **library crate** — not runnable on its own. Other crates depend on it.

Add to any crate's `Cargo.toml`:
```toml
[dependencies]
platform-shared = { workspace = true }
```

Then import:
```rust
use platform_shared::enums::DocumentStatus;
use platform_shared::s3_paths;
use platform_shared::constants::MAX_UPLOAD_SIZE_BYTES;
```

### Run Tests

```bash
cargo test -p platform-shared
```

## Why It Exists

Single source of truth for types shared between the API, database, and storage layers. When you add a new status or event, you change it here and all crates pick it up.
