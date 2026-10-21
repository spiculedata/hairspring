"""Sample instrumented programs, one per scenario: `python hs_sample.py <scenario> [args]`.

The tests (tests/test_process.py) and the Rust conformance test (crates/hairspring-props/tests/python_sdk.rs) run
them and inspect the events they write. `basic` has the same ids, messages, values and order as the C SDK's `basic`
scenario (sdks/c/tests/fixtures/hs_sample.c), so the two SDKs' reports can be compared line by line; the three sites
the C program never reaches are declared here, because Python cannot see a call site that has not run.
"""

from __future__ import annotations

import multiprocessing
import os
import signal
import subprocess
import sys
import threading
from collections.abc import Callable, Sequence

import hairspring as hs

THIS_FILE = os.path.abspath(__file__)
LONG_MSG = (
    "a deliberately long message so that each catalog line is large and two processes writing at "
    "once have a real chance of interleaving if a line were ever split across write calls; padding "
    "padding padding padding padding padding padding padding padding padding padding padding "
    "padding padding padding padding padding padding padding padding padding padding padding"
)
BIG_SITES = 64
BIG_ROUNDS = 3
ID_LIMIT = 128
MSG_LIMIT = 512


def basic(args: Sequence[str]) -> int:
    hs.declare(hs.Kind.REACHABLE, "basic.reach.never", "never reached")
    hs.declare(hs.Kind.UNREACHABLE, "basic.unreach.clean", "never reached either")
    hs.declare(hs.Kind.ALWAYS_OR_UNREACHABLE, "basic.aou.never", "never evaluated")
    for i in range(5):
        hs.always(True, "basic.always.ok", "always holds", i, 2)
    hs.always(False, "basic.always.bad", "always violated", 7, 8)
    for i in range(4):
        hs.sometimes(i % 2 == 0, "basic.sometimes", "sometimes holds", i)
    hs.reachable("basic.reach.hit", "is reached")
    hs.work(300)
    return 0


def merge_a(ok: bool) -> None:
    hs.sometimes(ok, "merge.shared", "one id, two sites", 1)


def merge_b(ok: bool) -> None:
    hs.sometimes(ok, "merge.shared", "one id, two sites", 2)


def merge(args: Sequence[str]) -> int:
    merge_a(True)
    merge_a(False)
    merge_b(True)
    merge_b(True)
    hs.reachable("merge.other", "a second, distinct id")
    return 0


def thread_body(index: int) -> None:
    hs.thread("w", index)
    hs.sometimes(True, "threads.each", "every thread evaluates", index)


def threads(args: Sequence[str]) -> int:
    count = int(args[0]) if args else 0
    hs.work(1)
    for i in range(count):
        worker = threading.Thread(target=thread_body, args=(i,))
        worker.start()
        worker.join()
    return 0


def exit_status(args: Sequence[str]) -> int:
    hs.reachable("exit.reach", "reached")
    hs.exit(3)
    return 0


def plain_exit(args: Sequence[str]) -> int:
    hs.reachable("plain.reach", "reached")
    sys.exit(7)


def raises(args: Sequence[str]) -> int:
    hs.reachable("raises.before", "reached before the exception")
    raise RuntimeError("deliberate")


def fork(args: Sequence[str]) -> int:
    for _ in range(2):
        hs.reachable("fork.parent", "parent reaches this")
    for _ in range(2):
        pid = os.fork()
        if pid == 0:
            for _ in range(3):
                hs.reachable("fork.child", "child reaches this")
            sys.exit(0)
        os.waitpid(pid, 0)
    return 0


def big(args: Sequence[str]) -> int:
    for rnd in range(BIG_ROUNDS):
        for site in range(BIG_SITES):
            hs.sometimes(rnd % 2 == 0, f"big.site.{site}", LONG_MSG, rnd)
    return 0


def spawn(args: Sequence[str]) -> int:
    hs.reachable("spawn.parent", "parent reaches this")
    done = subprocess.run([sys.executable, THIS_FILE, "merge"], check=False)
    return 0 if done.returncode == 0 else 3


def conflict(args: Sequence[str]) -> int:
    hs.always(True, "conflict.id", "declared as always")
    hs.sometimes(True, "conflict.id", "declared as sometimes")
    return 0


def empty(args: Sequence[str]) -> int:
    hs.work(1)
    return 0


def limits(args: Sequence[str]) -> int:
    hs.reachable("x" * ID_LIMIT, "m" * MSG_LIMIT)
    hs.reachable("bad id$with:odd/chars", 'quote " and backslash \\ in the message')
    return 0


def closefd(args: Sequence[str]) -> int:
    hs.reachable("closefd.reach", "reached")
    os.close(2)
    if args:
        fd = os.open(args[0], os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
        os.write(fd, b"application data\n")
    return 0


def sigpipe(args: Sequence[str]) -> int:
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    hs.reachable("sigpipe.reach", "reached")
    return 0


def on_signal(signum: int, frame: object) -> None:
    hs.reachable("signal.handler", "ran in a signal handler")


def signal_handler(args: Sequence[str]) -> int:
    signal.signal(signal.SIGUSR1, on_signal)
    for round_ in range(2000):
        hs.always(True, "signal.loop", "evaluated while signals arrive", round_)
        if round_ % 500 == 0:
            os.kill(os.getpid(), signal.SIGUSR1)
    return 0


def explicit(args: Sequence[str]) -> int:
    hs.reachable("explicit.before", "reached before the report")
    hs.report()
    hs.reachable("explicit.after", "reached after the report")
    return 0


def execself(args: Sequence[str]) -> int:
    hs.reachable("execself.before", "reached before the exec")
    os.execv(sys.executable, [sys.executable, THIS_FILE, "empty"])


def exec_after_report(args: Sequence[str]) -> int:
    hs.reachable("execafter.before", "reached before the exec")
    hs.report()
    os.execv(sys.executable, [sys.executable, THIS_FILE, "empty"])


def child_reaches() -> None:
    hs.reachable("mp.child", "the worker process reaches this")


def mp_spawn(args: Sequence[str]) -> int:
    hs.reachable("mp.parent", "parent reaches this")
    context = multiprocessing.get_context("spawn")
    process = context.Process(target=child_reaches)
    process.start()
    process.join()
    return process.exitcode or 0


def mp_fork(args: Sequence[str]) -> int:
    hs.reachable("mp.parent", "parent reaches this")
    context = multiprocessing.get_context("fork")
    process = context.Process(target=child_reaches)
    process.start()
    process.join()
    return process.exitcode or 0


SCENARIOS: dict[str, Callable[[Sequence[str]], int]] = {
    "basic": basic,
    "merge": merge,
    "threads": threads,
    "exit_status": exit_status,
    "plain_exit": plain_exit,
    "raises": raises,
    "fork": fork,
    "big": big,
    "spawn": spawn,
    "conflict": conflict,
    "empty": empty,
    "limits": limits,
    "closefd": closefd,
    "sigpipe": sigpipe,
    "signal_handler": signal_handler,
    "explicit": explicit,
    "execself": execself,
    "exec_after_report": exec_after_report,
    "mp_spawn": mp_spawn,
    "mp_fork": mp_fork,
}


def main(argv: Sequence[str]) -> int:
    if len(argv) < 2 or argv[1] not in SCENARIOS:
        print(f"usage: {argv[0]} <{'|'.join(SCENARIOS)}> [args]", file=sys.stderr)
        return 64
    return SCENARIOS[argv[1]](argv[2:])


if __name__ == "__main__":
    sys.exit(main(sys.argv))
