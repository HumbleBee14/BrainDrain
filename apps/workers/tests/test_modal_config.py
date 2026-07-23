import os

from src.config import WorkerSettings
from src.constants import MODAL_DEFAULT_GPU, MODAL_GPU_MAP


def _base_env(**over):
    env = {
        "APP_DATABASE_URL": "postgresql://u:p@localhost/db",
        "APP_S3_ACCESS_KEY": "k",
        "APP_S3_SECRET_KEY": "s",
    }
    env.update(over)
    return env


def test_modal_settings_defaults(monkeypatch):
    for k in list(os.environ):
        if k.startswith("APP_MODAL_"):
            monkeypatch.delenv(k, raising=False)
    for k, v in _base_env().items():
        monkeypatch.setenv(k, v)
    s = WorkerSettings()
    assert s.modal_app_name == "platform-training"
    assert s.modal_function_name == "train"
    assert s.modal_secret_name == "platform-training-secrets"
    assert s.modal_poll_interval_secs == 15


def test_modal_settings_env_override(monkeypatch):
    for k, v in _base_env(
        APP_MODAL_APP_NAME="my-app",
        APP_MODAL_POLL_INTERVAL_SECS="7",
    ).items():
        monkeypatch.setenv(k, v)
    s = WorkerSettings()
    assert s.modal_app_name == "my-app"
    assert s.modal_poll_interval_secs == 7


def test_gpu_map_has_known_classes():
    assert MODAL_GPU_MAP["a10080gb"] == "A100-80GB"
    assert MODAL_GPU_MAP["a10g"] == "A10G"
    assert MODAL_GPU_MAP["h100"] == "H100"
    assert MODAL_DEFAULT_GPU == "T4"
