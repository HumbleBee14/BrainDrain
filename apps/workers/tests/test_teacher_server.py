"""The teacher sidecar's lifecycle, which is the whole risk surface of on-policy.

A teacher that never becomes ready and a teacher that dies mid-run must both stop
the job. The failure this guards against is not a crash — it is a run that keeps
going without supervision and produces a model that looks trained.
"""

import signal
import subprocess

import pytest

from src.teacher import server as server_module
from src.teacher.server import (
    TeacherServer,
    TeacherServerConfig,
    TeacherServerError,
    split_devices,
    teacher_server,
)


class FakeProcess:
    """A subprocess stand-in whose exit is scripted.

    `poll_results` is consumed one call at a time; None means still running.
    """

    def __init__(self, poll_results=None):
        self._poll_results = list(poll_results or [])
        self._returncode = None
        self.signals: list[int] = []
        self.killed = False
        self.waits = 0
        self.wait_raises = 0

    def poll(self):
        if self._poll_results:
            self._returncode = self._poll_results.pop(0)
        return self._returncode

    def send_signal(self, sig):
        self.signals.append(sig)

    def kill(self):
        self.killed = True
        self._returncode = -9

    def wait(self, timeout=None):
        self.waits += 1
        if self.wait_raises > 0:
            self.wait_raises -= 1
            raise subprocess.TimeoutExpired(cmd="trl", timeout=timeout)
        self._returncode = self._returncode if self._returncode is not None else 0
        return self._returncode


def config(**overrides) -> TeacherServerConfig:
    base = {"model": "Qwen/Qwen3-32B", "devices": (0,), "port": 8123}
    return TeacherServerConfig(**{**base, **overrides})


def test_two_gpus_give_the_teacher_and_student_a_card_each():
    assert split_devices(2) == ((0,), (1,))


def test_extra_gpus_go_to_the_teacher():
    assert split_devices(4) == ((0, 1, 2), (3,))


def test_a_single_gpu_container_is_refused_not_shared():
    """Sharing one card between a 30B teacher and a student's optimizer state is
    the out-of-memory kill this topology exists to prevent."""
    with pytest.raises(TeacherServerError, match="at least 2 GPUs"):
        split_devices(1)


def test_command_names_the_model_and_pinned_revision():
    argv = config(revision="9216db57").command()

    assert argv[:2] == ["trl", "vllm-serve"]
    assert argv[argv.index("--model") + 1] == "Qwen/Qwen3-32B"
    assert argv[argv.index("--revision") + 1] == "9216db57"
    assert argv[argv.index("--port") + 1] == "8123"


def test_command_omits_revision_when_unpinned():
    assert "--revision" not in config().command()


def test_tensor_parallel_matches_the_teachers_device_count():
    argv = config(devices=(0, 1, 2)).command()

    assert argv[argv.index("--tensor_parallel_size") + 1] == "3"


def test_command_is_argv_so_a_model_id_cannot_become_shell_syntax():
    """Model ids come from tenant-owned dataset provenance."""
    argv = config(model="evil; rm -rf /").command()

    assert "evil; rm -rf /" in argv


def test_environment_pins_the_teacher_to_its_own_devices():
    env = config(devices=(0, 1)).environment(base={"PATH": "/usr/bin"})

    assert env["CUDA_VISIBLE_DEVICES"] == "0,1"
    assert env["PATH"] == "/usr/bin"


def test_ready_when_health_answers(monkeypatch):
    monkeypatch.setattr(server_module, "_probe", lambda url, timeout: True)
    server = TeacherServer(config(), FakeProcess())

    server.wait_until_ready(now=lambda: 0.0, sleep=lambda _: None)


def test_a_teacher_that_dies_at_boot_reports_its_exit_code(monkeypatch):
    """The common cause is a model too large for the devices it was given, and the
    exit code plus the model name is what makes that diagnosable."""
    monkeypatch.setattr(server_module, "_probe", lambda url, timeout: False)
    server = TeacherServer(config(), FakeProcess(poll_results=[1]))

    with pytest.raises(TeacherServerError, match="exited with code 1"):
        server.wait_until_ready(now=lambda: 0.0, sleep=lambda _: None)


def test_boot_times_out_rather_than_waiting_forever(monkeypatch):
    monkeypatch.setattr(server_module, "_probe", lambda url, timeout: False)
    server = TeacherServer(config(startup_timeout_secs=10), FakeProcess())
    clock = iter([0.0, 5.0, 20.0, 20.0])

    with pytest.raises(TeacherServerError, match="did not become ready"):
        server.wait_until_ready(now=lambda: next(clock), sleep=lambda _: None)


def test_check_alive_passes_while_the_teacher_runs():
    TeacherServer(config(), FakeProcess()).check_alive()


def test_check_alive_fails_the_run_when_the_teacher_is_gone():
    """TRL's client retries transient 5xx but nothing answers a dead port, so this
    is the check that turns a silent unsupervised run into a stopped one."""
    server = TeacherServer(config(), FakeProcess(poll_results=[137]))

    with pytest.raises(TeacherServerError, match="exited with code 137"):
        server.check_alive()


def test_stop_asks_politely_first_so_vllm_frees_gpu_memory():
    process = FakeProcess()
    TeacherServer(config(), process).stop()

    assert process.signals == [signal.SIGTERM]
    assert not process.killed


def test_stop_kills_a_teacher_that_ignores_sigterm():
    process = FakeProcess()
    process.wait_raises = 1
    TeacherServer(config(), process).stop()

    assert process.signals == [signal.SIGTERM]
    assert process.killed


def test_stop_is_a_noop_once_the_teacher_has_exited():
    process = FakeProcess(poll_results=[0])
    TeacherServer(config(), process).stop()

    assert process.signals == []


def test_the_teacher_is_stopped_even_when_training_raises(monkeypatch):
    """The teardown that matters: a training exception must not leave the teacher
    holding a GPU for the rest of the container's life."""
    monkeypatch.setattr(server_module.shutil, "which", lambda _: "/usr/bin/trl")
    monkeypatch.setattr(server_module, "_probe", lambda url, timeout: True)
    process = FakeProcess()

    with pytest.raises(RuntimeError, match="training blew up"):
        with teacher_server(config(), popen=lambda *a, **k: process):
            raise RuntimeError("training blew up")

    assert process.signals == [signal.SIGTERM]


def test_the_teacher_is_stopped_on_success(monkeypatch):
    monkeypatch.setattr(server_module.shutil, "which", lambda _: "/usr/bin/trl")
    monkeypatch.setattr(server_module, "_probe", lambda url, timeout: True)
    process = FakeProcess()

    with teacher_server(config(), popen=lambda *a, **k: process) as server:
        assert server.base_url == "http://127.0.0.1:8123"

    assert process.signals == [signal.SIGTERM]


def test_a_missing_trl_cli_fails_before_a_process_is_spawned(monkeypatch):
    """Running the on-policy strategy on the wrong image should say so, not spawn
    a doomed child and time out half an hour later."""
    monkeypatch.setattr(server_module.shutil, "which", lambda _: None)
    spawned = []

    with pytest.raises(TeacherServerError, match="on-policy image"):
        with teacher_server(config(), popen=lambda *a, **k: spawned.append(1)):
            pass

    assert spawned == []
