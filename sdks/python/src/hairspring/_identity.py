"""The process tag, and how it survives a spawn or an exec.

The tag travels in the environment as `HAIRSPRING_PROC`, with the owning pid and two flags in `HAIRSPRING_PROC_PID`
(`<pid>`, `<pid>:a`, `<pid>:u`, `<pid>:au`; `a` = the tag has been announced with a report, `u` = the tag is not stable
across runs):

* root process: `HAIRSPRING_PROC` if the harness set it, else `p1`;
* spawned child (`subprocess`, `multiprocessing` with `spawn`): the inherited pid is not its own, so the inherited tag
  belongs to an ancestor and the child takes `<inherited>.s<pid>`, flagged unstable (a pid differs between runs);
* exec without fork: the inherited pid IS its own and the tag was announced, so the image takes `<tag>.x1`.

Unlike the C SDK there is no fork handler: a forked child shares its parent's state and reports nothing of its own.
"""

from __future__ import annotations

from dataclasses import dataclass

from ._fmt import TAG_MAX, ident_hash, is_ident_char

ENV_TAG = "HAIRSPRING_PROC"
ENV_PID = "HAIRSPRING_PROC_PID"
DEFAULT_TAG = "p1"
RESERVED_TAG = "_eval"
# Readable part kept when a derived tag would pass 64 bytes.
HASH_PREFIX_MAX = 40
HASH_MIX = 0x9E3779B97F4A7C15
_MASK64 = (1 << 64) - 1


@dataclass(frozen=True)
class Identity:
    tag: str
    tag_stable: bool
    # The harness gave an invalid tag: it was refused, not shortened, and the SDK counts it as dropped.
    refused: bool


def tag_valid(tag: str) -> bool:
    """1 to 64 bytes of `[A-Za-z0-9._:/-]`, and not `_eval`, which is reserved for `eval` lines."""
    data = tag.encode("utf-8")
    return 0 < len(data) <= TAG_MAX and all(is_ident_char(b) for b in data) and tag != RESERVED_TAG


def derive(base: str, letter: str, n: int) -> str:
    """`base.<letter><n>`, or `<first 40 bytes of base>.h<16 hex digits>` when that would not fit in a tag."""
    plain = f"{base}.{letter}{n}"
    if len(plain) <= TAG_MAX:
        return plain
    keep = min(len(base), HASH_PREFIX_MAX)
    mix = (ident_hash(base, TAG_MAX) ^ ((n * HASH_MIX) & _MASK64) ^ ord(letter)) & _MASK64
    return f"{base[:keep]}.h{mix:016x}"


def owner_field(pid: int, announced: bool, stable: bool) -> str:
    """The value of `HAIRSPRING_PROC_PID`."""
    if not announced and stable:
        return str(pid)
    return f"{pid}:{'a' if announced else ''}{'' if stable else 'u'}"


@dataclass(frozen=True)
class _Owner:
    pid: int
    announced: bool
    unstable: bool


def _parse_owner(text: str) -> _Owner | None:
    """`<pid>` or `<pid>:` followed by the flags `a` and `u`; anything else is not an owner field."""
    digits = len(text) - len(text.lstrip("0123456789"))
    if digits == 0 or not text[:digits].isascii():
        return None
    pid, rest = int(text[:digits]), text[digits:]
    if not rest:
        return _Owner(pid, False, False)
    if rest[0] != ":" or any(c not in "au" for c in rest[1:]):
        return None
    return _Owner(pid, "a" in rest, "u" in rest)


def resolve(inherited_tag: str | None, owner: str | None, pid: int) -> Identity:
    """Decide the tag from the inherited environment values and this process's pid."""
    if not inherited_tag:
        return Identity(DEFAULT_TAG, True, False)
    if not tag_valid(inherited_tag):
        return Identity(DEFAULT_TAG, True, True)
    if owner is None:
        return Identity(inherited_tag, True, False)
    parsed = _parse_owner(owner)
    if parsed is not None and parsed.pid == pid:
        tag = derive(inherited_tag, "x", 1) if parsed.announced else inherited_tag
        return Identity(tag, not parsed.unstable, False)
    return Identity(derive(inherited_tag, "s", pid), False, False)
