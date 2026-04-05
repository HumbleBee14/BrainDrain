# Pre-Commit Hooks

**PR:** #23  
**Problem:** Forgetting to run `cargo fmt` or `ruff format` before committing
causes CI failures. This happened repeatedly and wastes time.

## How it works

A git pre-commit hook runs automatically before every `git commit`. It:

1. **Auto-formats Rust** — Runs `cargo fmt`, re-stages formatted files
2. **Auto-formats Python** — Runs `ruff format`, re-stages formatted files
3. **Checks Python lint** — Runs `ruff check`, blocks commit on errors
4. **Checks constant sync** — If you touched Rust constants or enums,
   verifies Python is in sync (auto-syncs if not)

You don't need to remember anything. Just `git commit` and the hook handles it.

## Setup (one-time per clone)

```bash
make setup-hooks
```

This runs `git config core.hooksPath .githooks`, telling git to use the
hooks in the `.githooks/` directory instead of the default `.git/hooks/`.

## Files

- `.githooks/pre-commit` — The hook script
- `Makefile` — `setup-hooks` target
