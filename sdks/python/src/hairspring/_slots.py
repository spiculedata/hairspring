"""The table of per-thread slots. A thread claims one on its first SDK call; slots are never recycled and keys cannot be
reused, so the table suits fixed pools. Past the 255th thread the rest share the last slot.

`SlotTable` does no locking of its own: the runtime calls it with its lock held."""

from __future__ import annotations

from dataclasses import dataclass

from . import _fmt

MAX_THREADS = 256
OVERFLOW_SLOT = MAX_THREADS - 1
OVERFLOW_KEY = "t#overflow"
MAX_INDEX = 2**32 - 1


class Slot:
    """One thread's counters. `evals` and `work` are changed only with the runtime lock held."""

    __slots__ = ("index", "key", "is_stable", "evals", "work")

    def __init__(self, index: int, key: str) -> None:
        self.index = index
        self.key = key
        self.is_stable = False
        self.evals = 0
        self.work = 0


@dataclass(frozen=True)
class ThreadView:
    """An immutable copy of a slot, for the report."""

    key: str
    evals: int
    work: int
    tape_consumed: int
    is_stable: bool


class SlotTable:
    def __init__(self) -> None:
        self._slots: list[Slot] = []
        self._claims = 0

    def used(self) -> int:
        return len(self._slots)

    def claim(self) -> tuple[Slot, bool]:
        """Claim the next slot; the flag says the thread shares the overflow slot."""
        n = self._claims
        self._claims += 1
        if n < OVERFLOW_SLOT:
            slot = Slot(n, f"t#{n + 1}")
            self._slots.append(slot)
            return slot, False
        if n == OVERFLOW_SLOT:
            self._slots.append(Slot(OVERFLOW_SLOT, OVERFLOW_KEY))
        return self._slots[OVERFLOW_SLOT], True

    def key_at(self, index: int) -> str:
        return self._slots[min(index, len(self._slots) - 1)].key if self._slots else ""

    def set_key(self, slot: Slot, label: str, index: int) -> bool:
        """Give `slot` the key `label-index`. Bytes of the label outside `[A-Za-z0-9._:/-]` become `_`. An empty label
        or a key over 64 bytes is a programming error (`ValueError`); a key another thread has, or a thread that shares
        the overflow slot, is refused with `False` and the thread keeps its claim-order key."""
        if not isinstance(label, str):
            raise TypeError("a thread label must be a string")
        if isinstance(index, bool) or not isinstance(index, int):
            raise TypeError("a thread index must be an integer")
        if not label:
            raise ValueError("a thread label must not be empty")
        if not 0 <= index <= MAX_INDEX:
            raise ValueError(f"a thread index must be between 0 and {MAX_INDEX}")
        key = f"{_fmt.sanitize_ident(label, len(label.encode('utf-8')))}-{index}"
        if len(key) > _fmt.THREAD_KEY_MAX:
            raise ValueError(f"the key {key!r} is longer than {_fmt.THREAD_KEY_MAX} bytes")
        if slot.index == OVERFLOW_SLOT:
            return False
        if any(other.key == key for other in self._slots if other is not slot):
            return False
        slot.key = key
        slot.is_stable = True
        return True

    def views(self) -> tuple[ThreadView, ...]:
        return tuple(ThreadView(s.key, s.evals, s.work, 0, s.is_stable) for s in self._slots)
