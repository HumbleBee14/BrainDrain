"""Stubs for driving `run_training_core` without a GPU, S3, or a real model.

Shared because more than one test needs to observe the order the core does things
in — the GPU reservation has to happen before the engine loads any weights, and
that is only visible from the core, not from any one function it calls.
"""

from src.activities.stubs import StartTrainingInput
from src.tenant_config import TenantLlmConfig


def fake_core_dependencies(monkeypatch, *, strategy) -> list[str]:
    """Replace the core's heavy collaborators. Returns the call-order log."""
    import src.activities.train_model as tm

    order: list[str] = []

    class _FakeEngine:
        def load_model(self, **_):
            order.append("load_model")
            return ("model", "tokenizer")

        def attach_adapter(self, model, **_):
            order.append("attach_adapter")
            return model

        def save_adapter(self, model, tokenizer, path):
            path.mkdir(parents=True, exist_ok=True)
            (path / "adapter.txt").write_text("x")

    monkeypatch.setattr(tm, "get_engine", lambda s, **_: _FakeEngine())
    monkeypatch.setattr(tm, "get_strategy", lambda key: strategy)
    monkeypatch.setattr(tm, "_download_dataset", lambda *a, **k: None)
    monkeypatch.setattr(tm, "_load_chatml_dataset", lambda p: [{"a": 1}])
    monkeypatch.setattr(tm, "_upload_adapter", lambda *a, **k: 123)
    monkeypatch.setattr(tm, "_get_metrics_collector", lambda s: None)
    return order


async def run_core(*, mode: str, hyperparams: dict):
    from src.activities.train_model import run_training_core

    class _Settings:
        s3_bucket = "b"
        training_engine = "transformers"

    return await run_training_core(
        StartTrainingInput(
            tenant_id="t",
            training_job_id="j",
            dataset_path="p",
            base_model="m",
            method="lora",
            mode=mode,
            hyperparams=hyperparams,
            gpu_class="a10080gb_dual",
        ),
        s3=object(),
        s3_bucket="b",
        settings=_Settings(),
        llm_config=TenantLlmConfig(
            api_base_url="http://x", api_key="k", model="m", max_tokens=10, is_custom=False
        ),
    )
