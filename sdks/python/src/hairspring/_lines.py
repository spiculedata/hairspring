"""One function per protocol line kind. Each returns the prefix, one JSON object and a newline as bytes; counters sit
in fixed-width fields so a line's length never depends on a counter's value. These are pure functions of immutable
values: nothing here reads the process."""

from __future__ import annotations

import enum
from dataclasses import dataclass

from . import _fmt
from ._kinds import Kind
from ._version import SDK_NAME, __version__

SDK_VERSION = __version__
PREFIX = "HAIRSPRING_EVENT "
# The protocol's per-process limit on ids; `end.sites` never claims more than a parser can accept.
MAX_IDS = 4096


class LineTooLong(Exception):
    """A line did not fit the protocol's cap; the caller counts it as dropped and never sends a truncated line."""


class EndReason(enum.Enum):
    EXIT = "exit"
    SIGNAL = "signal"
    EXPLICIT = "explicit"


@dataclass(frozen=True)
class ProcInfo:
    """What the lines say about the process."""

    tag: str
    tag_stable: bool
    signal_handlers: bool
    catalog_hash: int
    n_ids: int
    dropped: int


@dataclass(frozen=True)
class CatalogRow:
    id: str
    kind: Kind
    msg: str
    file: str
    line: int


@dataclass(frozen=True)
class WitnessOut:
    """A witness ready to print: the thread's key, that thread's own ordinal and four values."""

    key: str
    ord: int
    vals: tuple[int, int, int, int]


@dataclass(frozen=True)
class AssertRow:
    id: str
    kind: Kind
    trues: int
    falses: int
    first_true: WitnessOut | None
    first_false: WitnessOut | None


@dataclass(frozen=True)
class ThreadRow:
    key: str
    evals: int
    tape_consumed: int
    is_stable: bool


def _finish(body: str, max_len: int) -> bytes:
    line = (body + "}\n").encode("ascii")
    if len(line) > max_len:
        raise LineTooLong(f"a {len(line)} byte line passes the {max_len} byte cap")
    return line


def _open(kind: str) -> str:
    return f'{PREFIX}{{"v":1,"kind":"{kind}"'


def _proc(p: ProcInfo) -> str:
    return ',"proc":' + _fmt.ident(p.tag, _fmt.TAG_MAX)


def _bool(value: bool) -> str:
    return "true" if value else "false"


def hello(p: ProcInfo, max_len: int = _fmt.LINE_MAX) -> bytes:
    body = (
        _open("hello")
        + f',"sdk":"{SDK_NAME}","sdk_version":"{SDK_VERSION}"'
        + _proc(p)
        + f',"catalog_hash":"{_fmt.hex64(p.catalog_hash)}","signal_handlers":{_bool(p.signal_handlers)}'
        + ("" if p.tag_stable else ',"tag_stable":false')  # additive: a tag derived from a pid
    )
    return _finish(body, max_len)


def _file_tail(path: str) -> bytes:
    """The source path may be longer than the protocol allows; keep its tail, which names the file."""
    return path.encode("utf-8")[-_fmt.FILE_MAX :]


def _catalog_body(p: ProcInfo, row: CatalogRow, with_text: bool) -> str:
    body = (
        _open("catalog")
        + _proc(p)
        + ",\"id\":"
        + _fmt.ident(row.id, _fmt.ID_MAX)
        + f',"type":"{row.kind.wire_name}","must_hit":{_bool(row.kind.must_hit)}'
    )
    if not with_text:
        return body
    return (
        body
        + ',"msg":'
        + _fmt.json_text(row.msg, _fmt.MSG_MAX)
        + ',"loc":{"file":'
        + _fmt.json_text(_file_tail(row.file), _fmt.FILE_MAX)
        + f',"line":{row.line}}}'
    )


def catalog(p: ProcInfo, row: CatalogRow, max_len: int = _fmt.LINE_MAX) -> bytes:
    try:
        return _finish(_catalog_body(p, row, True), max_len)
    except LineTooLong:
        # An oversized message or path must not cost the whole line: retry with the text left out.
        return _finish(_catalog_body(p, row, False), max_len)


def _witness(w: WitnessOut | None, count: int) -> str:
    """A witness object padded to `WITNESS_WIDTH`, or `null` padded the same, so presence does not change length.
    A witness behind a zero count is never sent: the protocol rejects it."""
    if w is None or count == 0:
        text = "null"
    else:
        vals = ",".join(str(v) for v in w.vals)
        text = f'{{"thread":{_fmt.thread_key(w.key, _fmt.THREAD_KEY_MAX)},"ord":{_fmt.num_w(w.ord)},"vals":[{vals}]}}'
    return text + " " * max(_fmt.WITNESS_WIDTH - len(text), 0)


def assert_line(p: ProcInfo, row: AssertRow, max_len: int = _fmt.LINE_MAX) -> bytes:
    falses = 0 if row.kind.is_point else row.falses
    body = (
        _open("assert")
        + _proc(p)
        + ',"id":'
        + _fmt.ident(row.id, _fmt.ID_MAX)
        + f',"hits":{_fmt.num_w(row.trues + falses)},"trues":{_fmt.num_w(row.trues)},"falses":{_fmt.num_w(falses)}'
    )
    if not row.kind.is_point:
        body += ',"first_false":' + _witness(row.first_false, falses)
    body += ',"first_true":' + _witness(row.first_true, row.trues)
    return _finish(body, max_len)


def work(p: ProcInfo, ops: int, max_len: int = _fmt.LINE_MAX) -> bytes:
    return _finish(_open("work") + _proc(p) + f',"ops":{_fmt.num_w(ops)}', max_len)


def thread(p: ProcInfo, row: ThreadRow, max_len: int = _fmt.LINE_MAX) -> bytes:
    body = (
        _open("thread")
        + _proc(p)
        + ',"thread":'
        + _fmt.thread_key_padded(row.key)
        + f',"evals":{_fmt.num_w(row.evals)},"tape_consumed":{_fmt.num_w(row.tape_consumed)}'
        + (',"key_stable":true ' if row.is_stable else ',"key_stable":false')
    )
    return _finish(body, max_len)


def end(
    p: ProcInfo,
    reason: EndReason,
    status: int,
    evals: int,
    threads: int,
    max_len: int = _fmt.LINE_MAX,
) -> bytes:
    body = (
        _open("end")
        + _proc(p)
        + f',"reason":"{reason.value}","status":{_fmt.i64_w(status)},"sites":{min(p.n_ids, MAX_IDS)}'
        + f',"evals":{_fmt.num_w(evals)},"dropped":{_fmt.num_w(p.dropped)},"threads":{threads}'
    )
    return _finish(body, max_len)
