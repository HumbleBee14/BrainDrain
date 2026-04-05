#!/usr/bin/env python3
"""Generate Python constants and enums from the Rust source of truth.

Reads crates/shared/src/constants.rs and crates/shared/src/enums.rs,
then writes verified sections into apps/workers/src/constants.py.

Run after modifying Rust constants or enums:

    python scripts/sync_constants.py

Or add to CI to detect drift:

    python scripts/sync_constants.py --check
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RUST_CONSTANTS = REPO_ROOT / "crates" / "shared" / "src" / "constants.rs"
RUST_ENUMS = REPO_ROOT / "crates" / "shared" / "src" / "enums.rs"
PY_CONSTANTS = REPO_ROOT / "apps" / "workers" / "src" / "constants.py"

# Markers in the Python file that delimit auto-generated sections
CONSTANTS_START = "# ── AUTO-GENERATED FROM crates/shared/src/constants.rs ──"
CONSTANTS_END = "# ── END AUTO-GENERATED ──"
ENUMS_START = "# ── AUTO-GENERATED FROM crates/shared/src/enums.rs ──"
ENUMS_END = "# ── END AUTO-GENERATED ENUMS ──"


def parse_gpu_rates(rust_src: str) -> list[tuple[str, float]]:
    """Extract GPU_HOURLY_RATES entries from Rust source."""
    rates = []
    for match in re.finditer(r'\("(\w+)",\s*([\d.]+)\)', rust_src):
        rates.append((match.group(1), float(match.group(2))))
    return rates


def parse_default_rate(rust_src: str) -> float:
    """Extract GPU_DEFAULT_HOURLY_RATE from Rust source."""
    match = re.search(r'GPU_DEFAULT_HOURLY_RATE:\s*f64\s*=\s*([\d.]+)', rust_src)
    return float(match.group(1)) if match else 0.80


def _camel_to_snake(name: str) -> str:
    """Convert CamelCase to UPPER_SNAKE_CASE."""
    s1 = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s1).upper()


def _variant_value(variant: str) -> str:
    """Convert a Rust enum variant to its snake_case string value.

    Matches the Rust #[serde(rename_all = "snake_case")] behavior.
    """
    s1 = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", variant)
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s1).lower()


def parse_enums(rust_src: str) -> list[tuple[str, list[str]]]:
    """Extract all pub enum definitions and their variants from enums.rs.

    Only extracts simple enums (unit variants, no fields).
    """
    enums = []
    # Match: pub enum Name { ... }
    pattern = re.compile(r"pub enum (\w+)\s*\{([^}]+)\}", re.DOTALL)
    for m in pattern.finditer(rust_src):
        name = m.group(1)
        body = m.group(2)
        variants = []
        for line in body.splitlines():
            line = line.strip()
            # Skip comments and empty lines
            if not line or line.startswith("//") or line.startswith("#"):
                continue
            # Extract variant name (before any comma or comment)
            variant_match = re.match(r"^(\w+)", line)
            if variant_match:
                variants.append(variant_match.group(1))
        if variants:
            enums.append((name, variants))
    return enums


# Only generate Python classes for enums the workers actually need.
# Other enums (Plan, TeamRole, etc.) are API-only and not used by workers.
WORKER_ENUMS = {
    "DocumentStatus",
    "DatasetStatus",
    "TrainingJobStatus",
    "EvaluationStatus",
    "DeploymentStatus",
}


def generate_constants_section(rates: list[tuple[str, float]], default: float) -> str:
    """Generate the Python code block for GPU constants."""
    lines = [
        CONSTANTS_START,
        "# DO NOT EDIT MANUALLY — run: python scripts/sync_constants.py",
        "",
        "GPU_HOURLY_RATES: dict[str, float] = {",
    ]
    for name, rate in rates:
        lines.append(f'    "{name}": {rate},')
    lines.append("}")
    lines.append(f"GPU_DEFAULT_HOURLY_RATE: float = {default}")
    lines.append("")
    lines.append(CONSTANTS_END)
    return "\n".join(lines)


def generate_enums_section(enums: list[tuple[str, list[str]]]) -> str:
    """Generate the Python code block for enum classes."""
    lines = [
        ENUMS_START,
        "# DO NOT EDIT MANUALLY — run: python scripts/sync_constants.py",
    ]
    for name, variants in enums:
        if name not in WORKER_ENUMS:
            continue
        lines.append("")
        lines.append("")
        lines.append(f"class {name}:")
        for v in variants:
            lines.append(f'    {_camel_to_snake(v)} = "{_variant_value(v)}"')
    lines.append("")
    lines.append(ENUMS_END)
    return "\n".join(lines)


def replace_section(content: str, start: str, end: str, new_section: str) -> str:
    """Replace or append a delimited section in a file."""
    if start in content and end in content:
        before = content[: content.index(start)]
        after = content[content.index(end) + len(end) :]
        return before + new_section + after
    else:
        return content.rstrip() + "\n\n\n" + new_section + "\n"


def main():
    check_mode = "--check" in sys.argv

    # Parse Rust sources
    rust_constants_src = RUST_CONSTANTS.read_text(encoding="utf-8")
    rust_enums_src = RUST_ENUMS.read_text(encoding="utf-8")

    rates = parse_gpu_rates(rust_constants_src)
    default = parse_default_rate(rust_constants_src)
    enums = parse_enums(rust_enums_src)

    if not rates:
        print(f"ERROR: No GPU rates found in {RUST_CONSTANTS}", file=sys.stderr)
        sys.exit(1)

    worker_enums = [(n, v) for n, v in enums if n in WORKER_ENUMS]
    if not worker_enums:
        print(f"ERROR: No worker enums found in {RUST_ENUMS}", file=sys.stderr)
        sys.exit(1)

    constants_section = generate_constants_section(rates, default)
    enums_section = generate_enums_section(enums)

    # Build new file content
    current = PY_CONSTANTS.read_text(encoding="utf-8")

    new_content = replace_section(current, CONSTANTS_START, CONSTANTS_END, constants_section)
    new_content = replace_section(new_content, ENUMS_START, ENUMS_END, enums_section)

    if check_mode:
        if current == new_content:
            print("OK: Python constants and enums are in sync with Rust source.")
            sys.exit(0)
        else:
            print(
                "DRIFT DETECTED: Python constants/enums are out of sync with Rust.\n"
                "Run: python scripts/sync_constants.py",
                file=sys.stderr,
            )
            sys.exit(1)
    else:
        PY_CONSTANTS.write_text(new_content, encoding="utf-8")
        print(f"Synced {len(rates)} GPU rates + {len(worker_enums)} enums from Rust → Python")
        for name, rate in rates:
            print(f"  {name}: ${rate}/hr")
        print(f"  default: ${default}/hr")
        for name, variants in worker_enums:
            print(f"  {name}: {len(variants)} variants")


if __name__ == "__main__":
    main()
