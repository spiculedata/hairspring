"""Assertion sites: validation of what a call site passes, and the table that merges them.

A site is one (id, kind) pair. The same pair evaluated from several places is one site, so counts merge by
construction; an id used with two kinds is two sites (a catalog conflict the engine reports). `SiteTable` does no
locking of its own for changes: the runtime registers and counts with its lock held. `lookup` is a single dict read,
which is safe without the lock."""

from __future__ import annotations

import operator
from collections.abc import Sequence

from . import _fmt
from ._kinds import Kind

# What the catalog can hold; ids beyond it are refused and the caller counts them as dropped.
MAX_DISTINCT = 16384
MAX_VALS = 4
ZERO_VALS: tuple[int, int, int, int] = (0, 0, 0, 0)
_PADDING = ((), (0, 0, 0), (0, 0), (0,), ())
_INT64_SPAN = 1 << 64
_INT64_MAX = (1 << 63) - 1

Vals = tuple[int, int, int, int]
# (thread slot index, that thread's ordinal, the four values): the first evaluation on one side of a site.
WitnessRecord = tuple[int, int, Vals]


def check_id(raw: object) -> None:
    if not isinstance(raw, str):
        raise TypeError(f"an assertion id must be a string, not {type(raw).__name__}")
    size = len(raw.encode("utf-8"))
    if not 1 <= size <= _fmt.ID_MAX:
        raise ValueError(f"an assertion id is 1 to {_fmt.ID_MAX} bytes, not {size}")


def check_msg(raw: object) -> None:
    if not isinstance(raw, str):
        raise TypeError(f"an assertion message must be a string, not {type(raw).__name__}")
    size = len(raw.encode("utf-8"))
    if size > _fmt.MSG_MAX:
        raise ValueError(f"an assertion message is at most {_fmt.MSG_MAX} bytes, not {size}")


def _to_int64(value: int) -> int:
    wrapped = value % _INT64_SPAN
    return wrapped - _INT64_SPAN if wrapped > _INT64_MAX else wrapped


def pack_vals(vals: Sequence[object]) -> Vals:
    """Up to four integers, converted to signed 64 bits like a C cast, zero-filled. Called before any state changes, so
    a bad value counts nothing."""
    count = len(vals)
    if count == 0:
        return ZERO_VALS
    if count > MAX_VALS:
        raise TypeError(f"an assertion takes at most {MAX_VALS} values, not {count}")
    for v in vals:
        if type(v) is not int or not -_INT64_MAX <= v <= _INT64_MAX:
            return _pack_slow(vals)
    return (*vals, *_PADDING[count])  # type: ignore[return-value]


def _pack_slow(vals: Sequence[object]) -> Vals:
    """Booleans, other index types and integers outside the range that fits."""
    ints = [_to_int64(operator.index(v)) for v in vals]  # type: ignore[call-overload]
    ints.extend([0] * (MAX_VALS - len(ints)))
    return (ints[0], ints[1], ints[2], ints[3])


class Site:
    """One (id, kind) pair and its counters. `id` is the form that is sent."""

    __slots__ = ("id", "kind", "point", "msg", "file", "line", "trues", "falses", "first_true", "first_false")

    def __init__(self, id_: str, kind: Kind, msg: str, file: str, line: int) -> None:
        self.id = id_
        self.kind = kind
        self.point = kind.is_point
        self.msg = msg
        self.file = file
        self.line = line
        self.trues = 0
        self.falses = 0
        self.first_true: WitnessRecord | None = None
        self.first_false: WitnessRecord | None = None


class SiteTable:
    def __init__(self) -> None:
        self._raw: dict[tuple[str, Kind], Site] = {}
        self._by_sent: dict[tuple[str, Kind], Site] = {}
        self._ids: set[str] = set()
        self._order: list[Site] = []
        # The hot path's lookup, keyed by (the id as the caller wrote it, kind): one C-level dict read, no lock.
        self.find = self._raw.get

    def lookup(self, raw_id: str, kind: Kind) -> Site | None:
        return self.find((raw_id, kind))

    def register(self, kind: Kind, raw_id: str, msg: str, file: str, line: int) -> Site | None:
        """The site for this id and kind, made if it is new. `None` when the table is full of other ids. The first
        registration's message and location stand."""
        check_id(raw_id)
        check_msg(msg)
        sent = _fmt.sanitize_ident(raw_id, _fmt.ID_MAX)
        existing = self._by_sent.get((sent, kind))
        if existing is not None:
            self._raw[(raw_id, kind)] = existing
            return existing
        if sent not in self._ids and len(self._ids) >= MAX_DISTINCT:
            return None
        site = Site(sent, kind, msg, file, line)
        self._by_sent[(sent, kind)] = site
        self._raw[(raw_id, kind)] = site
        self._ids.add(sent)
        self._order.append(site)
        return site

    def all(self) -> tuple[Site, ...]:
        """Every site in registration order."""
        return tuple(self._order)

    def canonical(self) -> tuple[Site, ...]:
        """The first site of each id: the one whose counts the `assert` line carries."""
        seen: set[str] = set()
        out = []
        for site in self._order:
            if site.id not in seen:
                seen.add(site.id)
                out.append(site)
        return tuple(out)

    def distinct_ids(self) -> int:
        return len(self._ids)
