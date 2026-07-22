from pathlib import Path


def test_modal_app_wires_training_core():
    src = Path(__file__).resolve().parents[1] / "modal_app.py"
    text = src.read_text()
    # Uses Modal 1.0 local-source bundling, not the removed auto-mount.
    assert "add_local_python_source" in text
    # Calls the shared pure-compute core.
    assert "run_training_core" in text
    # 24h Modal max timeout for long training runs.
    assert "86400" in text
    # Async remote function (no event-loop blocking on the caller side via spawn).
    assert "async def train" in text
