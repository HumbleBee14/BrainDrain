# apps/workers/tests/test_dockerfiles.py
from pathlib import Path

W = Path(__file__).resolve().parents[1]


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
