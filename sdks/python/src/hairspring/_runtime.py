"""The process-wide state: sites, thread slots, the lock that guards them, and the report.

Every change to a counter happens with one lock held. The GIL does not make `count += 1` safe (a thread can be
switched out between the load and the store), so the lock is what makes counts exact under `threading`. It is also
taken by a `fork` (see `install_process_hooks`), so a child never inherits it held by a thread that no longer exists.
The critical sections are a few attribute updates; the report's lines are written outside the lock. The lock is
reentrant so that a signal handler or a finaliser that calls the SDK cannot hang the thread it interrupted."""

from __future__ import annotations

import atexit
import os
import sys
import threading
from collections.abc import Mapping, Sequence

from . import _identity
from ._catalog import Entry, catalog_hash
from ._identity import Identity
from ._kinds import Kind
from ._lines import AssertRow, CatalogRow, EndReason, WitnessOut
from ._output import Sink, open_sink
from ._report import Snapshot, write_report
from ._secure import is_privileged
from ._sites import UNTRACKED, ZERO_VALS, Site, SiteTable, WitnessRecord, pack_vals
from ._slots import Slot, SlotTable

_MASK64 = (1 << 64) - 1
UNKNOWN_FILE = "<unknown>"


def _where(levels: int) -> tuple[str, int]:
    """The file and line of the frame `levels` above the function that calls this one."""
    try:
        frame = sys._getframe(levels + 1)
    except ValueError:
        return UNKNOWN_FILE, 0
    return frame.f_code.co_filename, frame.f_lineno


class Runtime:
    """One process's recording and reporting. The module-level API uses a single instance; tests make their own."""

    def __init__(
        self,
        identity: Identity,
        sink: Sink | None,
        dropped: int = 0,
        owner_pid: int | None = None,
        export_env: bool = False,
    ) -> None:
        self._identity = identity
        self._sink = sink
        # Reentrant: a signal handler or a finaliser can call the SDK on the thread that is inside a critical section.
        # A plain lock would hang there; this costs, at worst, a witness taken from a half-updated site.
        self._lock = threading.RLock()
        self._local = threading.local()
        self._sites = SiteTable()
        self._slots = SlotTable()
        self._dropped = dropped
        self._status = 0
        self._reported = False
        # The process that started the SDK owns the report: a forked child shares this state and writes nothing.
        self._owner_pid = os.getpid() if owner_pid is None else owner_pid
        self._export_env = export_env

    # ---- starting up ------------------------------------------------------------------------------------------

    @classmethod
    def from_environment(cls, environ: Mapping[str, str] | None = None) -> Runtime:
        """The runtime for this process: the tag and the events channel come from the environment, and the exit and
        fork hooks are installed."""
        env = os.environ if environ is None else environ
        pid = os.getpid()
        identity = _identity.resolve(env.get(_identity.ENV_TAG), env.get(_identity.ENV_PID), pid)
        sink, opening_losses = open_sink(env, is_privileged())
        runtime = cls(
            identity,
            sink,
            dropped=opening_losses + int(identity.refused),
            owner_pid=pid,
            export_env=True,
        )
        runtime._export_identity(announced=False)
        runtime.install_process_hooks()
        return runtime

    def install_process_hooks(self) -> None:
        """Report at interpreter exit, and keep the lock safe across `fork`."""
        atexit.register(self._at_exit)
        os.register_at_fork(
            before=self._lock.acquire,
            after_in_parent=self._lock.release,
            after_in_child=self._lock.release,
        )

    def _export_identity(self, announced: bool) -> None:
        if not self._export_env:
            return
        pid = os.getpid()
        os.environ[_identity.ENV_TAG] = self._identity.tag
        os.environ[_identity.ENV_PID] = _identity.owner_field(pid, announced, self._identity.tag_stable)

    def _at_exit(self) -> None:
        self.report(EndReason.EXIT, None)

    # ---- state ------------------------------------------------------------------------------------------------

    @property
    def dropped(self) -> int:
        return self._dropped

    def _note_drop(self) -> None:
        with self._lock:
            self._dropped += 1

    def set_status(self, status: int) -> None:
        self._status = status

    def _claim_slot(self) -> Slot:
        with self._lock:
            slot, shared = self._slots.claim()
            if shared:
                self._dropped += 1  # a thread that shares the overflow slot
        self._local.slot = slot
        return slot

    def _slot(self) -> Slot:
        slot: Slot | None = getattr(self._local, "slot", None)
        return slot if slot is not None else self._claim_slot()

    def _site(self, kind: Kind, id_: str, msg: str, depth: int) -> Site | None:
        """The site for an id and kind, registered (with the caller's location) if it is new."""
        site = self._sites.lookup(id_, kind)
        if site is not None:
            return None if site is UNTRACKED else site
        file, line = _where(depth + 1)
        with self._lock:
            return self._sites.register(kind, id_, msg, file, line)

    # ---- the call sites ---------------------------------------------------------------------------------------

    def declare(self, kind: Kind, id_: str, msg: str = "", depth: int = 1) -> None:
        """Put a site in the catalog without evaluating it, so a site that is never reached still shows up (and a
        `reachable` that is never reached is reported as unmet). `depth` is how many frames above this call the
        source location is taken from: 1 is the caller."""
        if self._site(kind, id_, msg, depth) is None:
            with self._lock:
                self._dropped += 1

    def record(
        self,
        kind: Kind,
        ok: bool,
        id_: str,
        msg: str,
        vals: Sequence[int],
        depth: int = 1,
    ) -> None:
        """Count one evaluation. The first evaluation on each side of a site keeps its witness. A point kind
        (`reachable`, `unreachable`) has no condition: reaching it counts as true."""
        packed = pack_vals(vals) if vals else ZERO_VALS
        site = self._sites.find((id_, kind)) or self._site(kind, id_, msg, depth)
        try:
            slot: Slot = self._local.slot
        except AttributeError:
            slot = self._claim_slot()
        with self._lock:
            if site is None or site is UNTRACKED:
                self._dropped += 1  # more distinct ids than the catalog holds
                return
            slot.evals += 1
            record: WitnessRecord = (slot.index, slot.evals, packed)
            if ok or site.point:
                site.trues += 1
                if site.first_true is None:
                    site.first_true = record
            else:
                site.falses += 1
                if site.first_false is None:
                    site.first_false = record

    def work(self, n: int) -> None:
        """Add `n` operations to this thread's work count."""
        if isinstance(n, bool) or not isinstance(n, int):
            raise TypeError("work takes an integer")
        if n < 0:
            raise ValueError("work cannot be negative")
        slot = self._slot()
        with self._lock:
            slot.work = (slot.work + n) & _MASK64

    def thread(self, label: str, index: int) -> bool:
        """Give the calling thread the stable key `label-index`. See `SlotTable.set_key`."""
        slot = self._slot()
        with self._lock:
            return self._slots.set_key(slot, label, index)

    # ---- the report -------------------------------------------------------------------------------------------

    def _witness(self, record: WitnessRecord | None) -> WitnessOut | None:
        if record is None:
            return None
        slot_index, ord_, vals = record
        return WitnessOut(self._slots.key_at(slot_index), ord_, vals)

    def snapshot(self) -> Snapshot:
        """Everything the report says, read at one moment."""
        with self._lock:
            sites = self._sites.all()
            catalog = tuple(CatalogRow(s.id, s.kind, s.msg, s.file, s.line) for s in sites)
            asserts = tuple(
                AssertRow(
                    s.id,
                    s.kind,
                    s.trues,
                    s.falses,
                    self._witness(s.first_true),
                    self._witness(s.first_false),
                )
                for s in self._sites.canonical()
            )
            return Snapshot(
                catalog=catalog,
                asserts=asserts,
                threads=self._slots.views(),
                catalog_hash=catalog_hash(Entry(s.id, s.kind) for s in sites),
                n_ids=self._sites.distinct_ids(),
                dropped=self._dropped,
            )

    def report(self, reason: EndReason = EndReason.EXPLICIT, status: int | None = None) -> bool:
        """Write the report: at most once per process, and only from the process that started the SDK. Returns whether
        this call wrote it."""
        with self._lock:
            if self._reported:
                return False
            self._reported = True
        sink = self._sink
        if os.getpid() != self._owner_pid or sink is None:
            return False
        with sink.guarded():
            if sink.verify():
                self._note_drop()  # nothing is written: the descriptor leads elsewhere
            write_report(
                self.snapshot(),
                self._identity,
                sink,
                reason,
                self._status if status is None else status,
                self._note_drop,
            )
        self._export_identity(announced=True)
        return True
