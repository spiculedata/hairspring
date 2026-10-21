"""The five assertion kinds. The numbers are the C SDK's (`enum hs_kind`): they are part of the catalog hash."""

from __future__ import annotations

import enum


class Kind(enum.IntEnum):
    """What an assertion promises.

    | kind | trial-level failure | run-level UNMET |
    |---|---|---|
    | ALWAYS | any evaluation false | never evaluated |
    | ALWAYS_OR_UNREACHABLE | any evaluation false | none |
    | SOMETIMES | none | never true |
    | REACHABLE | none | never hit |
    | UNREACHABLE | hit at least once | none |
    """

    ALWAYS = 1
    ALWAYS_OR_UNREACHABLE = 2
    SOMETIMES = 3
    REACHABLE = 4
    UNREACHABLE = 5

    @property
    def wire_name(self) -> str:
        return self.name.lower()

    @property
    def must_hit(self) -> bool:
        return self in _MUST_HIT

    @property
    def is_point(self) -> bool:
        """A point has no condition: reaching it is the event (`reachable`, `unreachable`)."""
        return self in _POINTS


_MUST_HIT = frozenset({Kind.ALWAYS, Kind.SOMETIMES, Kind.REACHABLE})
_POINTS = frozenset({Kind.REACHABLE, Kind.UNREACHABLE})
