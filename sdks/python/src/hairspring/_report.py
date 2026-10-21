"""The report: an immutable snapshot of the runtime's state, and the lines written from it.

The snapshot is taken with the runtime lock held, so every count, witness and thread total in it belongs to one moment;
the lines are written afterwards without the lock."""

from __future__ import annotations

import dataclasses
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from . import _lines
from ._identity import Identity
from ._lines import AssertRow, CatalogRow, EndReason, ProcInfo, ThreadRow
from ._output import Sink
from ._slots import ThreadView


@dataclass(frozen=True)
class Snapshot:
    catalog: tuple[CatalogRow, ...]
    asserts: tuple[AssertRow, ...]
    threads: tuple[ThreadView, ...]
    catalog_hash: int
    n_ids: int
    dropped: int

    @property
    def work(self) -> int:
        return sum(t.work for t in self.threads)

    @property
    def evals(self) -> int:
        return sum(t.evals for t in self.threads)


class _Writer:
    """Sends lines and counts the ones that were lost. A line that does not fit, or cannot be written, is counted,
    never truncated."""

    def __init__(self, sink: Sink, note_drop: Callable[[], None]) -> None:
        self._sink = sink
        self._note_drop = note_drop
        self.lost = 0

    def send(self, build: Callable[..., bytes], *args: Any) -> None:
        try:
            written = self._sink.write_line(build(*args))
        except _lines.LineTooLong:
            written = False
        if not written:
            self.lost += 1
            self._note_drop()


def write_report(
    snapshot: Snapshot,
    identity: Identity,
    sink: Sink,
    reason: EndReason,
    status: int,
    note_drop: Callable[[], None],
) -> None:
    """Write hello, one catalog line per (id, kind), one assert per id, work, a thread line per thread, then end.
    `snapshot.dropped` is the loss count before the report; `end` adds the lines lost during it. `note_drop` is called
    for every lost line."""
    info = ProcInfo(
        tag=identity.tag,
        tag_stable=identity.tag_stable,
        signal_handlers=False,
        catalog_hash=snapshot.catalog_hash,
        n_ids=snapshot.n_ids,
        dropped=snapshot.dropped,
    )
    writer = _Writer(sink, note_drop)
    send = writer.send
    send(_lines.hello, info)
    for row in snapshot.catalog:
        send(_lines.catalog, info, row)
    for arow in snapshot.asserts:
        send(_lines.assert_line, info, arow)
    send(_lines.work, info, snapshot.work)
    for view in snapshot.threads:
        send(_lines.thread, info, ThreadRow(view.key, view.evals, view.tape_consumed, view.is_stable))
    closing = dataclasses.replace(info, dropped=info.dropped + writer.lost)
    send(_lines.end, closing, reason, status, snapshot.evals, len(snapshot.threads))
