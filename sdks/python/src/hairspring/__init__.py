"""Hairspring guest SDK for Python: in-source property assertions for programs under test.

A call site only counts in memory; the program reports once, at interpreter exit, in guest event protocol v1
(`docs/guest-protocol.md`). The standard library only; Linux and macOS; CPython 3.12.

    import hairspring as hs

    hs.always(balance >= 0, "bank.balance_non_negative", "no overdraft", balance)
    hs.sometimes(waited, "bank.writer.waited", "a writer waited for the lock")
    hs.reachable("bank.recovery.ran", "recovery ran")
    hs.unreachable("bank.bad_state", "never expected")
    hs.work(1)                      # operations done, reported as work.ops
    hs.thread("writer", 0)          # a stable thread key "writer-0"

Importing the package starts the SDK: it reads the environment, opens the events channel and registers the exit
report. A site is catalogued when it first runs, or earlier with `declare`.
"""

from __future__ import annotations

import sys
from typing import NoReturn

from ._kinds import Kind
from ._lines import EndReason
from ._runtime import Runtime
from ._version import SDK_NAME, __version__

__all__ = [
    "Kind",
    "SDK_NAME",
    "__version__",
    "always",
    "always_or_unreachable",
    "declare",
    "exit",
    "reachable",
    "report",
    "sometimes",
    "thread",
    "unreachable",
    "work",
]

_runtime = Runtime.from_environment()

# How many frames above `Runtime.record` or `Runtime.declare` the user's code is, from the functions below.
_CALLER = 2


def always(cond: object, id: str, msg: str = "", *vals: int) -> None:  # noqa: A002
    """`cond` must hold at every evaluation. Trial-level failure: any false. Run-level UNMET: never evaluated.

    `id` is 1 to 128 bytes (bytes outside `[A-Za-z0-9._:/-]` are sent as `_`); `msg` is at most 512 bytes; `vals` are up
    to four integers, captured at the first true and the first false evaluation (missing values are 0)."""
    _runtime.record(Kind.ALWAYS, bool(cond), id, msg, vals, _CALLER)


def always_or_unreachable(cond: object, id: str, msg: str = "", *vals: int) -> None:  # noqa: A002
    """Like `always`, but never reaching it is fine (no UNMET)."""
    _runtime.record(Kind.ALWAYS_OR_UNREACHABLE, bool(cond), id, msg, vals, _CALLER)


def sometimes(cond: object, id: str, msg: str = "", *vals: int) -> None:  # noqa: A002
    """`cond` must hold at least once per run. No trial-level failure; UNMET if never true."""
    _runtime.record(Kind.SOMETIMES, bool(cond), id, msg, vals, _CALLER)


def reachable(id: str, msg: str = "", *vals: int) -> None:  # noqa: A002
    """This point must be reached at least once per run. No trial-level failure; UNMET if never hit."""
    _runtime.record(Kind.REACHABLE, True, id, msg, vals, _CALLER)


def unreachable(id: str, msg: str = "", *vals: int) -> None:  # noqa: A002
    """This point must never be reached. Trial-level failure: hit at least once."""
    _runtime.record(Kind.UNREACHABLE, True, id, msg, vals, _CALLER)


def declare(kind: Kind, id: str, msg: str = "") -> None:  # noqa: A002
    """Put a site in the catalog without evaluating it. Python cannot see a call site that has not run, so a
    `reachable` or `sometimes` that is never reached would be invisible, and the run could not report it as unmet.
    Declare such sites once, at import time; evaluating the same id and kind later is the same site."""
    _runtime.declare(kind, id, msg, _CALLER)


def work(n: int) -> None:
    """Add `n` to the process's operation count, reported as `work.ops`."""
    _runtime.work(n)


def thread(label: str, index: int) -> bool:
    """Give the calling thread the key `label-index`. Returns False (and the thread keeps its claim-order key `t#<n>`,
    flagged `key_stable: false`) when another thread has that key or the thread shares the overflow slot. Raises
    `ValueError` for an empty label or a key over 64 bytes. Call it before the thread does anything else."""
    return _runtime.thread(label, index)


def report() -> None:
    """Write the report now (reason `explicit`). The first report wins; evaluations after it are not reported."""
    _runtime.report(EndReason.EXPLICIT, None)


def exit(status: int = 0) -> NoReturn:  # noqa: A001
    """Record `status` and exit through the normal path (`sys.exit`), so the exit report carries it."""
    _runtime.set_status(status)
    sys.exit(status)
