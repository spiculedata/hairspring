"""Feasibility probe for CPython threads (no SDK, no property): is this runtime's thread behaviour a function of the
scheduler's seed?

    python probe_py.py [--switch=1.0]

Four `threading.Thread`s share a `threading.Lock`, an unlocked check-then-act counter and a `queue.Queue`:

1. each takes the lock once and records its place in the order (the lock order);
2. each runs a loop that reads the shared counter, spins and writes it back with no lock (the racy counter: it gives
   the interpreter's lock switching something to interleave);
3. each puts a few items on the queue as it goes (the drain order is the runtime-specific structure the probe hashes);
4. each records its finishing position (the arrival order).

It prints exactly one line on stdout and exits 0:

    PROBE v1 lang=py lock_order=<16 hex> arrival=<16 hex> rand=<16 hex> vtime_ns=<20 digits> extra=<16 hex>
        racy_final=<20 digits> racy_expect=<20 digits> t_start_ns=<20 digits> t_end_ns=<20 digits>

(one line). `lock_order` and `arrival` are FNV-1a 64 hashes of the thread numbers in that order, `rand` is 64 bits from
the interpreter's random source (so an entropy leak shows as a difference between same-seed runs), `vtime_ns` is the
monotonic clock difference between start and end, and `extra` hashes the order the queue drains in. Those six fields
are the original contract.

The last four fields show that the interpreter really switched threads inside the racy window, which a determinism
check alone does not: `racy_final` is the unlocked counter after 4 threads x 200 read-spin-write rounds and
`racy_expect` is 800, so `racy_expect - racy_final` is the number of updates lost (0 means no thread switch ever
landed in a window); `t_start_ns` and `t_end_ns` are the monotonic clock before the threads start and after they are
joined (`vtime_ns` is their difference), so a clock that never advances, or never reaches the switch interval, shows.
`--switch=1.0` (the second profile) calls `sys.setswitchinterval(1.0)` before any thread starts. Usage error: exit 64.
"""

from __future__ import annotations

import os
import queue
import random
import sys
import threading
import time
from collections.abc import Sequence

THREADS = 4
ROUNDS = 200
SPIN_ROUNDS = 1000
RACY_EXPECTED = THREADS * ROUNDS
ITEMS_PER_THREAD = 3
SWITCH_ARGUMENT = "--switch=1.0"
SWITCH_INTERVAL_SECONDS = 1.0
EXIT_USAGE = 64
FNV_OFFSET = 0xCBF29CE484222325
FNV_PRIME = 0x00000100000001B3
MASK64 = (1 << 64) - 1
SPIN_MULTIPLIER = 6364136223846793005


def fnv1a64(data: bytes) -> int:
    value = FNV_OFFSET
    for byte in data:
        value = ((value ^ byte) * FNV_PRIME) & MASK64
    return value


def hash_of(items: Sequence[str]) -> int:
    return fnv1a64("".join(items).encode("ascii"))


def spin(rounds: int) -> int:
    acc = 0
    for i in range(rounds):
        acc = (acc * SPIN_MULTIPLIER + i) & MASK64
    return acc


class Counter:
    def __init__(self) -> None:
        self.value = 0


class Probe:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.lock_order: list[str] = []
        self.arrival_lock = threading.Lock()
        self.arrival: list[str] = []
        self.counter = Counter()
        self.sink = Counter()
        self.items: queue.Queue[str] = queue.Queue()
        self.barrier = threading.Barrier(THREADS)

    def worker(self, index: int) -> None:
        self.barrier.wait()
        with self.lock:
            self.lock_order.append(str(index))
        puts_every = max(ROUNDS // ITEMS_PER_THREAD, 1)
        for round_ in range(ROUNDS):
            seen = self.counter.value  # check ...
            self.sink.value = spin(SPIN_ROUNDS)
            self.counter.value = seen + 1  # ... then act, with no lock
            if round_ % puts_every == 0:
                self.items.put(f"{index}{round_ // puts_every}")
        with self.arrival_lock:
            self.arrival.append(str(index))

    def drained(self) -> list[str]:
        out = []
        while True:
            try:
                out.append(self.items.get_nowait())
            except queue.Empty:
                return out


def line(probe: Probe, rand: int, start_ns: int, end_ns: int) -> bytes:
    text = (
        f"PROBE v1 lang=py lock_order={hash_of(probe.lock_order):016x} arrival={hash_of(probe.arrival):016x} "
        f"rand={rand:016x} vtime_ns={end_ns - start_ns:020d} extra={hash_of(probe.drained()):016x} "
        f"racy_final={probe.counter.value:020d} racy_expect={RACY_EXPECTED:020d} "
        f"t_start_ns={start_ns:020d} t_end_ns={end_ns:020d}\n"
    )
    return text.encode("ascii")


def write_stdout(data: bytes) -> None:
    view = memoryview(data)
    while view:
        view = view[os.write(1, view) :]


def main(argv: Sequence[str]) -> int:
    args = list(argv[1:])
    if args not in ([], [SWITCH_ARGUMENT]):
        print(f"usage: {argv[0]} [{SWITCH_ARGUMENT}]", file=sys.stderr)
        return EXIT_USAGE
    if args:
        sys.setswitchinterval(SWITCH_INTERVAL_SECONDS)
    probe = Probe()
    started = time.monotonic_ns()
    threads = [threading.Thread(target=probe.worker, args=(i,)) for i in range(THREADS)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    ended = time.monotonic_ns()
    write_stdout(line(probe, random.getrandbits(64), started, ended))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
