"""Was this process started with elevated rights? If so, `HAIRSPRING_EVENTS_FILE` is not trusted: the SDK would open a
path from the environment for writing.

On Linux the kernel is asked (`AT_SECURE` in `/proc/self/auxv`): a set-uid or set-gid binary sets it, and so do file
capabilities and a security-module transition, which change no id. Elsewhere real and effective ids are compared."""

from __future__ import annotations

import os
import struct
import sys
from collections.abc import Callable

AT_NULL = 0
AT_SECURE = 23
AUXV_PATH = "/proc/self/auxv"
_WORD_FORMAT = "Q" if struct.calcsize("P") == 8 else "I"
_PAIR = struct.Struct("=" + _WORD_FORMAT * 2)


def read_auxv() -> bytes:
    with open(AUXV_PATH, "rb") as f:
        return f.read()


def at_secure_from(raw: bytes) -> tuple[bool, bool]:
    """Read the auxiliary vector: pairs of machine words (type, value), ended by type 0. Returns (found, secure)."""
    for offset in range(0, len(raw) - _PAIR.size + 1, _PAIR.size):
        kind, value = _PAIR.unpack_from(raw, offset)
        if kind == AT_NULL:
            break
        if kind == AT_SECURE:
            return True, value != 0
    return False, False


def ids_differ() -> bool:
    return os.getuid() != os.geteuid() or os.getgid() != os.getegid()


def is_privileged_with(read: Callable[[], bytes], differ: Callable[[], bool], linux: bool) -> bool:
    """Decide from the auxiliary vector when it can. An ordinary Linux process can always read its own vector; a
    file-capability binary that is not dumpable may not be able to, and that is the case `AT_SECURE` exists for, so on
    Linux a vector that cannot be read means privileged. A vector that reads but has no `AT_SECURE` entry (an old
    kernel) and every other platform fall back to comparing ids."""
    try:
        found, secure = at_secure_from(read())
    except OSError:
        if linux:
            return True
        return differ()
    return secure if found else differ()


def is_privileged() -> bool:
    return is_privileged_with(read_auxv, ids_differ, sys.platform.startswith("linux"))
