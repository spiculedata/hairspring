"""Helpers shared by the tests: an in-memory sink, a parser for event lines, and a runner for the sample programs."""

from __future__ import annotations

import contextlib
import json
import os
import subprocess
import sys
from collections.abc import Iterator, Mapping, Sequence
from dataclasses import dataclass
from typing import Any

from hairspring._identity import Identity
from hairspring._runtime import Runtime

PREFIX = "HAIRSPRING_EVENT "
SDK_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC_DIR = os.path.join(SDK_DIR, "src")
SAMPLE = os.path.join(SDK_DIR, "samples", "hs_sample.py")
QUICKSTART = os.path.join(SDK_DIR, "examples", "quickstart.py")
ENV_KEYS_TO_CLEAR = (
    "HAIRSPRING_PROC",
    "HAIRSPRING_PROC_PID",
    "HAIRSPRING_EVENTS",
    "HAIRSPRING_EVENTS_FILE",
)


class MemorySink:
    """Collects lines instead of writing them. `fail` makes every write fail."""

    def __init__(self, fail: bool = False, lost: bool = False) -> None:
        self.lines: list[bytes] = []
        self.fail = fail
        self.lost = lost
        self.closed = False

    def write_line(self, data: bytes) -> bool:
        if self.fail:
            return False
        self.lines.append(data)
        return True

    def verify(self) -> bool:
        lost, self.lost = self.lost, False
        return lost

    @contextlib.contextmanager
    def guarded(self) -> Iterator[None]:
        yield

    def close(self) -> None:
        self.closed = True

    def events(self) -> list[dict[str, Any]]:
        return parse_events(b"".join(self.lines))

    def of_kind(self, kind: str) -> list[dict[str, Any]]:
        return [e for e in self.events() if e["kind"] == kind]


def parse_events(data: bytes) -> list[dict[str, Any]]:
    events = []
    for raw in data.decode("utf-8").splitlines():
        if raw.startswith(PREFIX):
            events.append(json.loads(raw[len(PREFIX) :]))
    return events


def root_identity(tag: str = "p1", stable: bool = True) -> Identity:
    return Identity(tag=tag, tag_stable=stable, refused=False)


def make_runtime(
    sink: MemorySink | None = None, dropped: int = 0, owner_pid: int | None = None
) -> tuple[Runtime, MemorySink]:
    sink = sink if sink is not None else MemorySink()
    return Runtime(root_identity(), sink, dropped=dropped, owner_pid=owner_pid), sink


@dataclass(frozen=True)
class Run:
    returncode: int
    stdout: bytes
    stderr: bytes

    def events(self) -> list[dict[str, Any]]:
        return parse_events(self.stderr)


def clean_env(extra: Mapping[str, str] | None = None) -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if k not in ENV_KEYS_TO_CLEAR}
    env["PYTHONPATH"] = SRC_DIR
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    env.update(extra or {})
    return env


def run_script(
    script: str,
    args: Sequence[str] = (),
    env: Mapping[str, str] | None = None,
    **kwargs: Any,
) -> Run:
    done = subprocess.run(
        [sys.executable, script, *args],
        env=clean_env(env),
        capture_output=True,
        timeout=120,
        check=False,
        **kwargs,
    )
    return Run(done.returncode, done.stdout, done.stderr)


def run_sample(scenario: str, *args: str, env: Mapping[str, str] | None = None) -> Run:
    return run_script(SAMPLE, [scenario, *args], env)
