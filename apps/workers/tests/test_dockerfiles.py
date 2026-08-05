# apps/workers/tests/test_dockerfiles.py
import re
import tomllib
from pathlib import Path

W = Path(__file__).resolve().parents[1]


def _pyproject() -> dict:
    with (W / "pyproject.toml").open("rb") as handle:
        return tomllib.load(handle)


def _extra(name: str) -> dict[str, str]:
    """Exact pins in an optional-dependency group, as {package: version}."""
    specs = _pyproject()["project"]["optional-dependencies"][name]
    return dict(
        match.groups()
        for spec in specs
        if (match := re.fullmatch(r"([A-Za-z0-9_.-]+)==([0-9][^,;\s]*)", spec.strip()))
    )


def _modal_constant(name: str) -> str:
    body = (W / "modal_app.py").read_text()
    match = re.search(rf'^{name} = "([^"]+)"', body, re.MULTILINE)
    assert match, f"{name} is not defined in modal_app.py"
    return match.group(1)


def test_default_image_installs_modal_client_not_ml():
    df = (W / "Dockerfile").read_text()
    assert "--extra gpu-cloud" in df  # can dispatch to cloud
    assert "--extra ml" not in df  # stays slim (no torch/unsloth)


def test_gpu_image_installs_ml_and_cuda():
    df = (W / "Dockerfile.gpu").read_text()
    assert "--extra ml" in df
    assert "cuda" in df.lower()


def _package_name(spec: str) -> str:
    """Strip version/extras from a requirement spec: 'boto3>=1.36' -> 'boto3'."""
    for sep in (">=", "==", "<=", "~=", ">", "<", "["):
        spec = spec.split(sep)[0]
    return spec.strip().strip('"').lower()


def _pyproject_runtime_deps() -> set[str]:
    text = (W / "pyproject.toml").read_text()
    block = text.split("dependencies = [", 1)[1].split("]", 1)[0]
    return {
        _package_name(line)
        for raw in block.splitlines()
        if (line := raw.strip()) and not line.startswith("#")
    }


def test_modal_image_covers_pyproject_runtime_deps():
    """A dep missing from the Modal image only fails at GPU-call time.

    modal_app.py imports the full worker module graph remotely, so any runtime
    dependency absent from its image surfaces as ModuleNotFoundError inside the
    training container rather than at deploy or import time.
    """
    modal_app = (W / "modal_app.py").read_text()
    installed = {_package_name(tok) for tok in modal_app.split('"') if tok.strip()}
    missing = sorted(dep for dep in _pyproject_runtime_deps() if dep not in installed)
    assert not missing, f"Modal image is missing runtime deps: {missing}"


def test_modal_pins_match_pyproject():
    """pyproject.toml declares the pins; modal_app.py repeats them as literals
    because Modal builds image layers from this file at deploy time and the module
    is also imported in the container, where pyproject.toml is absent. That
    duplication is only safe while something fails when the two disagree."""
    assert _modal_constant("TRL_UNSLOTH_VERSION") == _extra("ml")["trl"]
    assert _modal_constant("TRL_ON_POLICY_VERSION") == _extra("on-policy")["trl"]
    assert _modal_constant("VLLM_EXTRACTION_VERSION") == _extra("extraction")["vllm"]
    assert _modal_constant("VLLM_ON_POLICY_VERSION") == _extra("on-policy")["vllm"]


def test_unsloth_stack_keeps_the_trl_version_unsloth_allows():
    """unsloth constrains trl to <=0.24.0. Raising the `ml` extra to the on-policy
    version makes the training image unresolvable at build time — which is why
    on-policy needs an image of its own rather than a version bump."""
    unsloth_trl = _extra("ml")["trl"]
    on_policy_trl = _extra("on-policy")["trl"]

    assert tuple(int(p) for p in unsloth_trl.split(".")) <= (0, 24, 0), (
        f"ml pins trl {unsloth_trl}, above what unsloth permits"
    )
    assert unsloth_trl != on_policy_trl, (
        "the two stacks share a trl pin; one of them cannot resolve"
    )


def test_trl_is_pinned_exactly_wherever_it_appears():
    """The trainers we use live in trl.experimental, which reserves the right to
    change without notice, and TRL has already deleted a trainer between the
    versions an open range would span."""
    for group in ("ml", "on-policy"):
        specs = _pyproject()["project"]["optional-dependencies"][group]
        trl = [spec for spec in specs if _package_name(spec) == "trl"]
        assert trl, f"trl missing from the '{group}' extra"
        assert all("==" in spec for spec in trl), f"trl is not pinned in '{group}': {trl}"


def test_the_two_vllm_pins_stay_distinct_and_bounded():
    """Extraction's pin carries its measured logprob contract; the on-policy pin is
    bounded by what TRL supports. Collapsing them silently breaks one or the other."""
    extraction = _extra("extraction")["vllm"]
    on_policy = _extra("on-policy")["vllm"]

    assert extraction != on_policy, (
        "extraction and on-policy share a vLLM pin; one of them is now running a "
        "version its contract was not established against"
    )
    assert tuple(int(p) for p in on_policy.split(".")) <= (0, 25, 1), (
        f"on-policy vLLM {on_policy} exceeds the ceiling TRL {_extra('on-policy')['trl']} allows"
    )


def test_on_policy_stack_excludes_unsloth():
    """Unsloth pins its own torch build and cannot resolve alongside vLLM, which
    the on-policy trainer needs in-process to reach the teacher."""
    packages = {
        _package_name(spec)
        for spec in _pyproject()["project"]["optional-dependencies"]["on-policy"]
    }
    assert "unsloth" not in packages


def _beam_app() -> str:
    return (W / "beam_app.py").read_text()


def test_beam_pins_match_modal_app():
    """beam_app.py repeats modal_app's training-image literals (it cannot import
    modal_app inside a Beam container). Same duplication rule: safe only while
    disagreement fails a test."""
    match = re.search(r'^TRL_UNSLOTH_VERSION = "([^"]+)"', _beam_app(), re.MULTILINE)
    assert match, "TRL_UNSLOTH_VERSION is not defined in beam_app.py"
    assert match.group(1) == _modal_constant("TRL_UNSLOTH_VERSION")


def test_beam_image_covers_pyproject_runtime_deps():
    installed = {_package_name(tok) for tok in _beam_app().split('"') if tok.strip()}
    missing = sorted(dep for dep in _pyproject_runtime_deps() if dep not in installed)
    assert not missing, f"Beam image is missing runtime deps: {missing}"


def test_modal_images_mount_every_src_data_file():
    """add_local_python_source ships only .py files, so any data file an
    activity reads must have its directory mounted explicitly — otherwise it
    surfaces as FileNotFoundError inside the GPU container (found live with
    the evaluation benchmarks, 2026-08-05)."""
    modal_app = (W / "modal_app.py").read_text()
    data_dirs = {
        str(p.parent.relative_to(W))
        for p in (W / "src").rglob("*")
        if p.is_file() and p.suffix != ".py" and "__pycache__" not in p.parts
    }
    unmounted = sorted(d for d in data_dirs if d not in modal_app)
    assert not unmounted, f"Modal images do not mount data dirs: {unmounted}"
