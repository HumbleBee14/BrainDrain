# packages/shared-types

**TypeScript type definitions mirroring the Rust backend DTOs and enums.**

| | |
|---|---|
| Language | TypeScript |
| Type | Internal npm package (`@platform/shared-types`) |
| Used by | `apps/web` |
| Mirrors | `crates/shared/src/enums.rs` + `crates/api/src/dto/` |

## What's Inside

| File | Purpose |
|---|---|
| `enums.ts` | All status enums: `DocumentStatus`, `TrainingJobStatus`, `DeploymentStatus`, `TaskType`, etc. |
| `api.ts` | API response types: `ProjectResponse`, `DocumentResponse`, `TrainingJobResponse`, `ModelResponse`, etc. |
| `index.ts` | Re-exports everything |

## Why It Exists

Keeps the frontend type-safe against the Rust API. When you add a new field or enum variant in Rust, update it here so TypeScript catches mismatches at compile time.

## Sync Rule

**Rust is the source of truth.** These types must always match:
- `enums.ts` ↔ `crates/shared/src/enums.rs`
- `api.ts` ↔ `crates/api/src/dto/*.rs`

## Usage

```bash
pnpm build        # Compile to dist/
pnpm type-check   # Verify types
```
