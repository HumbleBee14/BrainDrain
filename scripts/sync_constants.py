#!/usr/bin/env python3
"""Generate Python constants from the Rust source of truth.

Reads crates/shared/src/constants.rs and writes a verified section
into apps/workers/src/constants.py. Run after modifying Rust constants:

    python scripts/sync_constants.py

Or add to CI to detect drift:

    python scripts/sync_constants.py --check
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RUST_CONSTANTS = REPO_ROOT / "crates" / "shared" / "src" / "constants.rs"
PY_CONSTANTS = REPO_ROOT / "apps" / "workers" / "src" / "constants.py"

# Markers in the Python file that delimit the auto-generated section
START_MARKER = "# ── AUTO-GENERATED FROM crates/shared/src/constants.rs ──"
END_MARKER = "# ── END AUTO-GENERATED ──"


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


def generate_python_section(rates: list[tuple[str, float]], default: float) -> str:
    """Generate the Python code block for constants.py."""
    lines = [
        START_MARKER,
        "# DO NOT EDIT MANUALLY — run: python scripts/sync_constants.py",
        "",
        "GPU_HOURLY_RATES: dict[str, float] = {",
    ]
    for name, rate in rates:
        lines.append(f'    "{name}": {rate},')
    lines.append("}")
    lines.append(f"GPU_DEFAULT_HOURLY_RATE: float = {default}")
    lines.append("")
    lines.append(END_MARKER)
    return "\n".join(lines)


def update_python_file(py_path: Path, new_section: str) -> str:
    """Replace the auto-generated section in the Python file."""
    content = py_path.read_text(encoding="utf-8")

    if START_MARKER in content and END_MARKER in content:
        # Replace existing section
        before = content[: content.index(START_MARKER)]
        after = content[content.index(END_MARKER) + len(END_MARKER) :]
        return before + new_section + after
    else:
        # Append (first run)
        return content.rstrip() + "\n\n\n" + new_section + "\n"


def main():
    check_mode = "--check" in sys.argv

    rust_src = RUST_CONSTANTS.read_text(encoding="utf-8")
    rates = parse_gpu_rates(rust_src)
    default = parse_default_rate(rust_src)

    if not rates:
        print(f"ERROR: No GPU rates found in {RUST_CONSTANTS}", file=sys.stderr)
        sys.exit(1)

    new_section = generate_python_section(rates, default)
    new_content = update_python_file(PY_CONSTANTS, new_section)

    if check_mode:
        current = PY_CONSTANTS.read_text(encoding="utf-8")
        if current == new_content:
            print("OK: Python constants are in sync with Rust source.")
            sys.exit(0)
        else:
            print(
                "DRIFT DETECTED: Python constants are out of sync with Rust.\n"
                "Run: python scripts/sync_constants.py",
                file=sys.stderr,
            )
            sys.exit(1)
    else:
        PY_CONSTANTS.write_text(new_content, encoding="utf-8")
        print(f"Synced {len(rates)} GPU rates from Rust → Python")
        for name, rate in rates:
            print(f"  {name}: ${rate}/hr")
        print(f"  default: ${default}/hr")


if __name__ == "__main__":
    main()
