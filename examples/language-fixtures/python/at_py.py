"""Python lost-update fixture: a planted race, its locked twin and a wider-window arm, in one program.

    python at_py.py buggy|fixed|wide [--switch=1.0]

Four threads each do 1,000 times: read a shared counter, run a busy loop of pure Python (no system call, no sleep, no
SDK call), write back the value read plus one. The read and the write are separate operations, so an update is lost
when another thread runs between them (CPython hands the interpreter lock to another thread every few milliseconds,
inside the busy loop). `fixed` holds a `threading.Lock` across the same three steps. `wide` is `buggy` with a ten
times longer busy loop. The mode is an argument, never a build flag, so one file identifies what ran in every arm.
`--switch=1.0` (allowed only as the second argument) sets the interpreter's thread switch interval to one second
before any thread starts.

It reports through the Hairspring Python SDK: `py.counter_exact` (`always`, the final counter is 4,000) and
`py.all_started` (`sometimes`), each evaluated once in the main thread after the workers are joined. Stdout gets one
line, in one write:

    LANGAT v1 lang=py mode=<mode> final=<20 digits> expect=<20 digits> order=<16 hex> rand=<16 hex>

Exit status: 0 whatever the property says; 3 if the program itself failed (a thread did not start, the barrier broke,
the arrival list is not four entries long); 64 for a usage error. The verdict is the SDK's report, not the exit code.
"""

from __future__ import annotations

import os
import random
import sys
import threading
from collections.abc import Sequence
from dataclasses import dataclass


def _find_sdk() -> None:
    """The SDK sits next to this file when the two are staged together, else in the repository's `sdks/python/src`.
    (`python -I` does not put this file's directory on the import path.)"""
    here = os.path.dirname(os.path.abspath(__file__))
    repository_copy = os.path.normpath(os.path.join(here, "..", "..", "..", "sdks", "python", "src"))
    for directory in (repository_copy, here):
        if os.path.isdir(os.path.join(directory, "hairspring")):
            sys.path.insert(0, directory)


_find_sdk()

import hairspring as hs  # noqa: E402

LANG = "py"
THREADS = 4
ITERATIONS = 1000
WINDOW = 1000
WINDOW_WIDE = 10000
EXPECTED = THREADS * ITERATIONS
SWITCH_ARGUMENT = "--switch=1.0"
SWITCH_INTERVAL_SECONDS = 1.0
EXIT_INVARIANT = 3
EXIT_USAGE = 64
FNV_OFFSET = 0xCBF29CE484222325
FNV_PRIME = 0x00000100000001B3
MASK64 = (1 << 64) - 1
SPIN_MULTIPLIER = 6364136223846793005


@dataclass(frozen=True)
class Config:
    mode: str
    spin_rounds: int
    locked: bool
    switch: bool


MODES = {
    "buggy": (WINDOW, False),
    "fixed": (WINDOW, True),
    "wide": (WINDOW_WIDE, False),
}


def parse_args(argv: Sequence[str]) -> Config | None:
    """The configuration for `argv[1:]`, or None for a usage error."""
    args = list(argv[1:])
    if not args or args[0] not in MODES or len(args) > 2:
        return None
    switch = len(args) == 2
    if switch and args[1] != SWITCH_ARGUMENT:
        return None
    spin_rounds, locked = MODES[args[0]]
    return Config(args[0], spin_rounds, locked, switch)


class Counter:
    """The shared counter. Plain attribute reads and writes: two separate operations, never an atomic update."""

    def __init__(self) -> None:
        self.counter = 0


class Sink:
    """Where the busy loop's result goes, so the loop has an effect."""

    def __init__(self) -> None:
        self.value = 0


class Shared:
    def __init__(self) -> None:
        self.state = Counter()
        self.sink = Sink()
        self.lock = threading.Lock()
        self.started = 0
        self.started_lock = threading.Lock()
        self.arrival: list[int] = []
        self.arrival_lock = threading.Lock()
        self.failures: list[str] = []
        # A plain barrier: a worker that fails (or cannot start) aborts it, which wakes the others, so nothing waits on a
        # clock.
        self.barrier = threading.Barrier(THREADS)


def spin(rounds: int) -> int:
    """A busy loop in this file's own code: `rounds` multiply-adds on an accumulator the caller keeps."""
    acc = 0
    for i in range(rounds):
        acc = (acc * SPIN_MULTIPLIER + i) & MASK64
    return acc


def racy_iterations(state: Counter, sink: Sink, spin_rounds: int) -> None:
    for _ in range(ITERATIONS):
        v = state.counter
        acc = spin(spin_rounds)
        state.counter = v + 1
        sink.value = acc
        hs.work(1)


def locked_iterations(state: Counter, sink: Sink, spin_rounds: int, lock: threading.Lock) -> None:
    for _ in range(ITERATIONS):
        with lock:
            v = state.counter
            acc = spin(spin_rounds)
            state.counter = v + 1
        sink.value = acc
        hs.work(1)


def worker(index: int, config: Config, shared: Shared) -> None:
    try:
        hs.thread("worker", index)
        with shared.started_lock:
            shared.started += 1
        shared.barrier.wait()
        if config.locked:
            locked_iterations(shared.state, shared.sink, config.spin_rounds, shared.lock)
        else:
            racy_iterations(shared.state, shared.sink, config.spin_rounds)
        with shared.arrival_lock:
            shared.arrival.append(index)
    except BaseException as error:  # reported as an invariant failure by the main thread
        shared.failures.append(f"worker {index}: {type(error).__name__}: {error}")
        shared.barrier.abort()


def fnv1a64(data: bytes) -> int:
    value = FNV_OFFSET
    for byte in data:
        value = ((value ^ byte) * FNV_PRIME) & MASK64
    return value


def order_hash(arrival: Sequence[int]) -> int:
    """FNV-1a 64 of the arrival list written as ASCII digits."""
    return fnv1a64("".join(str(i) for i in arrival).encode("ascii"))


def result_line(config: Config, final: int, order: int, rand: int) -> bytes:
    text = (
        f"LANGAT v1 lang={LANG} mode={config.mode} final={final:020d} expect={EXPECTED:020d} "
        f"order={order:016x} rand={rand:016x}\n"
    )
    return text.encode("ascii")


def write_stdout(data: bytes) -> None:
    """One write call for the line (a short write is completed)."""
    view = memoryview(data)
    while view:
        view = view[os.write(1, view) :]


def run_workers(config: Config, shared: Shared) -> None:
    threads = [threading.Thread(target=worker, args=(i, config, shared), name=f"worker-{i}") for i in range(THREADS)]
    started = []
    for thread in threads:
        try:
            thread.start()
        except RuntimeError as error:
            shared.failures.append(f"a thread did not start: {error}")
            shared.barrier.abort()
            break
        started.append(thread)
    for thread in started:
        thread.join()


def main(argv: Sequence[str]) -> int:
    config = parse_args(argv)
    if config is None:
        print(f"usage: {argv[0]} buggy|fixed|wide [{SWITCH_ARGUMENT}]", file=sys.stderr)
        hs.exit(EXIT_USAGE)  # through the SDK, so the end line carries the same status as the process
    if config.switch:
        sys.setswitchinterval(SWITCH_INTERVAL_SECONDS)
    shared = Shared()
    run_workers(config, shared)
    if shared.failures or len(shared.arrival) != THREADS:
        print("invariant failure: " + "; ".join(shared.failures or ["the arrival list is incomplete"]), file=sys.stderr)
        hs.exit(EXIT_INVARIANT)
    final = shared.state.counter
    hs.always(final == EXPECTED, f"{LANG}.counter_exact", "counter equals T*K", final, EXPECTED, THREADS, ITERATIONS)
    hs.sometimes(shared.started == THREADS, f"{LANG}.all_started", "all workers started", shared.started)
    try:
        write_stdout(result_line(config, final, order_hash(shared.arrival), random.getrandbits(64)))
    except OSError as error:
        print(f"invariant failure: the stdout line could not be written: {error}", file=sys.stderr)
        hs.exit(EXIT_INVARIANT)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
