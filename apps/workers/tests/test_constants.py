"""Constants the workers hold that must agree with the Rust control plane.

`constants.py` has an auto-generated half (synced from `enums.rs`) and a
hand-maintained half. These tests cover the hand-maintained half, where drift is
silent: a GPU class the API will happily admit but the worker cannot map falls
back to the default GPU rather than failing, and the job runs on hardware nobody
asked for.
"""

import re
from pathlib import Path

from src.constants import (
    GPU_DEFAULT_DEVICE_COUNT,
    GPU_DEVICE_COUNTS,
    MODAL_DEFAULT_GPU,
    MODAL_GPU_MAP,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
ENUMS_RS = REPO_ROOT / "crates/shared/src/enums.rs"
CONSTANTS_RS = REPO_ROOT / "crates/shared/src/constants.rs"


def rust_gpu_class_names() -> set[str]:
    """The wire names of `GpuClass`, read from the Rust rate table.

    The table is keyed by the same snake_case strings strum derives for the enum,
    and `every_gpu_class_has_a_rate` on the Rust side asserts the table is
    complete — so parsing it here needs no CamelCase inflection rules.
    """
    body = CONSTANTS_RS.read_text(encoding="utf-8")
    table = re.search(r"GPU_HOURLY_RATES[^=]*=\s*&\[(.*?)\];", body, re.DOTALL)
    assert table, "could not find GPU_HOURLY_RATES in the Rust constants"
    return set(re.findall(r'\("([a-z0-9_]+)",', table.group(1)))


def test_every_rust_gpu_class_maps_to_a_modal_gpu():
    unmapped = sorted(rust_gpu_class_names() - set(MODAL_GPU_MAP))
    assert unmapped == [], (
        f"GPU classes the API can admit but the worker cannot map: {unmapped}. "
        f"These would silently run on {MODAL_DEFAULT_GPU}."
    )


def test_no_modal_mapping_without_a_rust_class():
    """A mapping with no class behind it is dead config that reads as supported."""
    extra = sorted(set(MODAL_GPU_MAP) - rust_gpu_class_names())
    assert extra == [], f"Modal GPU mappings with no GpuClass variant: {extra}"


def test_multi_device_classes_request_more_than_one_device():
    """`device_count() == 2` in Rust has to mean ":2" to Modal, or the container
    comes up with one card and the teacher lands on the student's GPU."""
    for name, modal_gpu in MODAL_GPU_MAP.items():
        if name.endswith("_dual"):
            assert modal_gpu.endswith(":2"), (
                f"{name} maps to '{modal_gpu}', which requests a single device"
            )
        else:
            assert ":" not in modal_gpu, f"{name} unexpectedly requests multiple devices"


def test_device_counts_agree_with_what_is_provisioned():
    """Billing splits an on-policy container's cost by device count, so a count
    that disagrees with the hardware actually requested misprices the teacher's
    share — and the teacher-GPU spend cap is what reads that share."""
    for name, modal_gpu in MODAL_GPU_MAP.items():
        provisioned = int(modal_gpu.split(":")[1]) if ":" in modal_gpu else 1
        declared = GPU_DEVICE_COUNTS.get(name, GPU_DEFAULT_DEVICE_COUNT)
        assert declared == provisioned, (
            f"{name} provisions {provisioned} device(s) but bills as {declared}"
        )


def test_no_device_count_without_a_gpu_class():
    extra = sorted(set(GPU_DEVICE_COUNTS) - rust_gpu_class_names())
    assert extra == [], f"device counts for classes that do not exist: {extra}"


def test_declared_gpu_classes_are_lowercase_wire_names():
    for name in MODAL_GPU_MAP:
        assert name == name.lower(), f"{name} would never match a lowercased gpu_class"


def test_enums_and_constants_sources_exist():
    """Guards the test itself: a moved Rust file would otherwise pass vacuously."""
    assert ENUMS_RS.exists(), f"expected the shared enums at {ENUMS_RS}"
    assert CONSTANTS_RS.exists(), f"expected the shared constants at {CONSTANTS_RS}"
