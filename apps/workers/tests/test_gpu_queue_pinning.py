from pathlib import Path

WF = Path(__file__).resolve().parents[1] / "src" / "workflows"


def test_evaluate_pins_gpu_queue():
    assert 'task_queue="ml-pipeline-gpu"' in (WF / "evaluate.py").read_text()


def test_export_pins_gpu_queue():
    assert 'task_queue="ml-pipeline-gpu"' in (WF / "export.py").read_text()
