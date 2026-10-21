"""The catalog's summary values: how many distinct ids, and the order-independent hash of the distinct (id, kind)
pairs that `hello.catalog_hash` carries. The hash equals the C SDK's for the same ids and kinds."""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass

from ._fmt import ID_MAX, ident_hash
from ._kinds import Kind

KIND_SHIFT = 56
HASH_MIX = 0x9E3779B97F4A7C15
_MASK64 = (1 << 64) - 1


@dataclass(frozen=True)
class Entry:
    id: str
    kind: Kind


def pair_hash(entry: Entry) -> int:
    return (((ident_hash(entry.id, ID_MAX) ^ (int(entry.kind) << KIND_SHIFT)) & _MASK64) * HASH_MIX) & _MASK64


def catalog_hash(entries: Iterable[Entry]) -> int:
    """The sum of the pair hashes, so the order of the entries does not matter. Entries must be distinct pairs."""
    return sum(pair_hash(e) for e in entries) & _MASK64


def distinct_ids(entries: Iterable[Entry]) -> int:
    return len({e.id for e in entries})
