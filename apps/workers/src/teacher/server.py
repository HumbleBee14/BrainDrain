"""The teacher server an on-policy run trains against.

On-policy distillation needs the teacher live: the student writes an answer and
the teacher scores the tokens the student actually chose, which no precomputed
artifact can answer. TRL reaches a remote teacher through its own
`/get_sequence_logprobs/` endpoint, which only `trl vllm-serve` exposes — a plain
OpenAI-compatible vLLM server does not have it.

The teacher therefore runs as a **subprocess of the trainer**, on its own GPU
inside the same container, reachable on loopback. That is a deliberate choice
over a separately scheduled server, for two reasons that outweigh the
flexibility lost:

- **A leaked teacher GPU cannot happen.** The teacher is a child process. When
  the trainer returns, crashes, or is killed, the container goes with it and the
  child dies. There is no state where a teacher outlives the run that pays for
  it, so there is nothing for a reaper to find.
- **TRL's client sends no authentication.** A routable teacher endpoint would be
  an open logprob oracle for the tenant's teacher weights. Loopback is not
  reachable from outside the container.

What this file does own is the failure matrix that remains real: a teacher that
never becomes ready, and a teacher that dies mid-run. Both must fail the job
loudly, because a run that silently continues without its teacher produces a
model that looks trained and learned nothing.
"""

from __future__ import annotations

import logging
import os
import shutil
import signal
import socket
import subprocess
import time
import urllib.error
import urllib.request
from contextlib import contextmanager
from dataclasses import dataclass, field

logger = logging.getLogger("platform.teacher.server")

DEFAULT_HOST = "127.0.0.1"

# `trl vllm-serve` has to load tens of gigabytes of weights before it answers, and
# a cold weight cache pulls them over the network first. Generous because the
# alternative to waiting is discarding a GPU-minute-expensive container.
DEFAULT_STARTUP_TIMEOUT_SECS = 1800
HEALTH_POLL_INTERVAL_SECS = 5.0

# Grace given to SIGTERM before SIGKILL. vLLM frees GPU memory on shutdown, which
# matters when the trainer keeps running in the same container afterwards.
SHUTDOWN_GRACE_SECS = 30

TEACHER_UNAVAILABLE_MESSAGE = (
    "The teacher model could not be reached, so this run was stopped rather than "
    "trained without it. Nothing was charged for training time that produced no model."
)


def reserve_loopback_port(host: str = DEFAULT_HOST) -> int:
    """A free port on loopback, claimed by binding it and letting it go.

    Not a fixed port, because a fixed port is a way to train against the wrong
    teacher: where two runs share a machine — which the local provider allows —
    the second one's health probe answers from the first one's teacher, and the
    student is then graded by a model that has nothing to do with its dataset.
    Nothing in the readiness check can tell the difference, since `trl vllm-serve`
    does not report which model it is holding.
    """
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.bind((host, 0))
        return int(probe.getsockname()[1])


class TeacherServerError(RuntimeError):
    """The teacher could not be started, or stopped being available.

    Always terminal for the run: retrying inside the same container would train
    against a teacher we already know is gone.
    """


@dataclass
class TeacherServerConfig:
    """How to boot the teacher beside the trainer.

    `devices` is the set of GPU ordinals the teacher may use, and the trainer must
    use the complement — the two share a container, not a card.

    `port` defaults to a freshly reserved one rather than a well-known number, so
    that two runs on one machine cannot end up talking to each other's teacher.
    """

    model: str
    revision: str | None = None
    devices: tuple[int, ...] = (0,)
    host: str = DEFAULT_HOST
    port: int = field(default_factory=reserve_loopback_port)
    dtype: str = "bfloat16"
    gpu_memory_utilization: float = 0.90
    max_model_len: int | None = None
    startup_timeout_secs: int = DEFAULT_STARTUP_TIMEOUT_SECS
    extra_args: tuple[str, ...] = field(default_factory=tuple)

    @property
    def base_url(self) -> str:
        return f"http://{self.host}:{self.port}"

    def command(self) -> list[str]:
        """The `trl vllm-serve` invocation, as an argv list.

        Never a shell string: the model id comes from a tenant's dataset
        provenance, and argv keeps it a single argument no matter what it holds.
        """
        argv = [
            "trl",
            "vllm-serve",
            "--model",
            self.model,
            "--host",
            self.host,
            "--port",
            str(self.port),
            "--dtype",
            self.dtype,
            "--gpu_memory_utilization",
            str(self.gpu_memory_utilization),
            "--tensor_parallel_size",
            str(len(self.devices)),
        ]
        if self.revision:
            argv += ["--revision", self.revision]
        if self.max_model_len is not None:
            argv += ["--max_model_len", str(self.max_model_len)]
        argv += list(self.extra_args)
        return argv

    def environment(self, base: dict[str, str] | None = None) -> dict[str, str]:
        """Child environment pinning the teacher to its own GPUs.

        Without this the teacher grabs device 0 — the same card the trainer is
        about to load the student onto — and one of the two dies out of memory.
        """
        env = dict(os.environ if base is None else base)
        env["CUDA_VISIBLE_DEVICES"] = ",".join(str(d) for d in self.devices)
        return env


def _nvml_device_count() -> int:
    """Physical GPU count, read without initializing CUDA.

    Deliberately not `torch.cuda.device_count()`: that caches its answer and, on
    the way to it, makes the process's device set permanent. Everything about the
    split has to be decided before any CUDA state exists, because
    `CUDA_VISIBLE_DEVICES` stops having any effect the moment it does.
    """
    try:
        import pynvml
    except ImportError as exc:
        raise TeacherServerError(
            "Cannot enumerate GPUs: pynvml is missing, so this is not the on-policy image."
        ) from exc

    pynvml.nvmlInit()
    try:
        return int(pynvml.nvmlDeviceGetCount())
    finally:
        pynvml.nvmlShutdown()


def container_gpu_ids(env: dict[str, str] | None = None, count_devices=_nvml_device_count):
    """GPU ordinals this process is allowed to use.

    Honours an inherited `CUDA_VISIBLE_DEVICES` rather than assuming the container
    owns every card the driver can see. A scheduler that hands out `2,3` on an
    8-GPU host means device 2 and device 3 — splitting `range(8)` there would
    address six cards belonging to other jobs and one of them would be a teacher.

    Returns the ids as CUDA will interpret them in a child process: absolute, so
    passing a subset straight back down to `CUDA_VISIBLE_DEVICES` selects the same
    physical cards.
    """
    declared = (env if env is not None else os.environ).get("CUDA_VISIBLE_DEVICES")
    if declared is None or not declared.strip():
        return tuple(range(count_devices()))

    entries = [entry.strip() for entry in declared.split(",") if entry.strip()]
    if not all(entry.isdigit() for entry in entries):
        # GPU-UUID and MIG forms are legal here and cannot be split by ordinal.
        raise TeacherServerError(
            f"CUDA_VISIBLE_DEVICES is set to '{declared}', which names devices by "
            f"identifier rather than index. On-policy distillation needs to assign "
            f"cards to the teacher and the student by index."
        )
    return tuple(int(entry) for entry in entries)


def split_devices(device_ids) -> tuple[tuple[int, ...], tuple[int, ...]]:
    """Partition a container's GPUs into (teacher, student) ordinals.

    The teacher takes the leading devices and the student the last one. Raises on
    a single-device container: sharing one card between a 30B-class teacher and a
    student's training state is the failure this whole arrangement exists to
    avoid, and it must not degrade quietly into an out-of-memory kill.
    """
    ids = tuple(device_ids)
    if len(ids) < 2:
        raise TeacherServerError(
            f"On-policy distillation needs at least 2 GPUs in the container, found "
            f"{len(ids)}. The teacher and the student cannot share one card."
        )
    return ids[:-1], ids[-1:]


def _probe(url: str, timeout: float) -> bool:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:  # noqa: S310
            return 200 <= response.status < 300
    except (urllib.error.URLError, TimeoutError, OSError):
        return False


class TeacherServer:
    """A running `trl vllm-serve` child, with the health of it observable.

    Not a context manager by itself — `teacher_server()` below owns the lifetime,
    so that no caller can hold a reference to a teacher that was never awaited or
    never stopped.
    """

    def __init__(self, config: TeacherServerConfig, process: subprocess.Popen):
        self.config = config
        self._process = process

    @property
    def base_url(self) -> str:
        return self.config.base_url

    def is_running(self) -> bool:
        return self._process.poll() is None

    def exit_code(self) -> int | None:
        return self._process.poll()

    def check_alive(self) -> None:
        """Raise if the teacher has died. Called between training steps.

        A dead teacher is the case TRL's HTTP retries cannot save: its client
        retries transient 5xx, but nothing answers a port whose process exited.
        """
        code = self.exit_code()
        if code is not None:
            raise TeacherServerError(
                f"The teacher server exited with code {code} during training. "
                f"{TEACHER_UNAVAILABLE_MESSAGE}"
            )

    def wait_until_ready(self, *, now=time.monotonic, sleep=time.sleep) -> None:
        """Block until the teacher answers `/health/`, or fail.

        Polls the process as well as the port: a teacher that dies at boot (out of
        memory on a too-large model is the common one) is reported as what it is,
        instead of as a timeout half an hour later.
        """
        deadline = now() + self.config.startup_timeout_secs
        health_url = f"{self.base_url}/health/"

        while True:
            code = self.exit_code()
            if code is not None:
                raise TeacherServerError(
                    f"The teacher server exited with code {code} before becoming "
                    f"ready. Check that {self.config.model} fits on "
                    f"{len(self.config.devices)} GPU(s) at {self.config.dtype}."
                )

            if _probe(health_url, timeout=HEALTH_POLL_INTERVAL_SECS):
                # Something is listening — confirm it is our child. A teacher that
                # lost the race for its port dies immediately, and whatever won the
                # race answers a health check just as happily.
                code = self.exit_code()
                if code is not None:
                    raise TeacherServerError(
                        f"The teacher server exited with code {code} while its port "
                        f"answered, so {self.base_url} belongs to another process."
                    )
                logger.info("Teacher server ready at %s", self.base_url)
                return

            if now() >= deadline:
                raise TeacherServerError(
                    f"The teacher server did not become ready within "
                    f"{self.config.startup_timeout_secs}s at {self.base_url}."
                )

            sleep(HEALTH_POLL_INTERVAL_SECS)

    def stop(self) -> None:
        """Terminate the teacher and reclaim its GPU memory.

        Best-effort by design: this runs on the way out of a run that has already
        either succeeded or failed, and the container is about to be reclaimed
        regardless. A teacher that will not die is not worth failing a completed
        training run over — but it is worth a loud log line.
        """
        if not self.is_running():
            return

        self._process.send_signal(signal.SIGTERM)
        try:
            self._process.wait(timeout=SHUTDOWN_GRACE_SECS)
            logger.info("Teacher server stopped")
            return
        except subprocess.TimeoutExpired:
            logger.warning("Teacher server ignored SIGTERM after %ss; killing", SHUTDOWN_GRACE_SECS)

        self._process.kill()
        try:
            self._process.wait(timeout=SHUTDOWN_GRACE_SECS)
        except subprocess.TimeoutExpired:
            logger.error("Teacher server survived SIGKILL; the container will reclaim it")


@contextmanager
def teacher_server(config: TeacherServerConfig, *, popen=subprocess.Popen):
    """Run the teacher for the duration of the block.

    The teardown is in a `finally`, so it happens on success, on a training
    exception, and on cancellation alike. Combined with the container dying when
    this process does, that is the whole of the teacher's lifecycle — there is no
    third path where a teacher stays up.
    """
    if shutil.which("trl") is None:
        raise TeacherServerError(
            "The `trl` CLI is not installed in this image, so no teacher can be "
            "served. On-policy distillation requires the on-policy image."
        )

    logger.info(
        "Starting teacher %s on GPU(s) %s",
        config.model,
        config.environment().get("CUDA_VISIBLE_DEVICES"),
    )
    # Output is inherited rather than captured: when a teacher dies at boot the
    # reason (almost always an out-of-memory line from vLLM) is in its own stderr,
    # and swallowing that leaves nothing but an exit code to debug from.
    process = popen(config.command(), env=config.environment())
    server = TeacherServer(config, process)
    try:
        server.wait_until_ready()
        yield server
    finally:
        server.stop()
