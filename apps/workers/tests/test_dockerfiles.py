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
