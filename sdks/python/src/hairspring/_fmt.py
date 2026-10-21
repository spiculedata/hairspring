"""Pure formatting for the guest protocol: the widths and padding match the C SDK, so a line's length never depends
on a counter's value. Everything returns ASCII text."""

from __future__ import annotations

LINE_MAX = 4096
NUM_WIDTH = 20
ID_MAX = 128
THREAD_KEY_MAX = 64
MSG_MAX = 512
FILE_MAX = 256
TAG_MAX = 64
# Widest witness object: thread key 64 bytes, ord 20 digits, four 20-character values.
WITNESS_WIDTH = 197

_MASK64 = (1 << 64) - 1
_FNV_OFFSET = 0xCBF29CE484222325
_FNV_PRIME = 0x00000100000001B3
_ALPHABET = frozenset(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._:/-")
_THREAD_ALPHABET = _ALPHABET | {ord("#")}
_PRINTABLE = range(0x20, 0x7F)
_REPLACEMENT = ord("_")


def num_w(value: int) -> str:
    """An unsigned counter right-aligned in 20 characters."""
    return f"{value & _MASK64:>{NUM_WIDTH}}"


def i64_w(value: int) -> str:
    """A signed counter right-aligned in 20 characters, the sign inside the field."""
    return f"{value:>{NUM_WIDTH}}"


def hex64(value: int) -> str:
    """Sixteen lowercase hex digits."""
    return f"{value & _MASK64:016x}"


def is_ident_char(byte: int) -> bool:
    return byte in _ALPHABET


def _bytes_of(text: str | bytes) -> bytes:
    return text if isinstance(text, bytes) else text.encode("utf-8")


def _until_nul(data: bytes) -> bytes:
    end = data.find(0)
    return data if end < 0 else data[:end]


def json_text(text: str | bytes, max_bytes: int) -> str:
    """A quoted JSON string of at most `max_bytes` source bytes. Printable ASCII survives, `"` and `\\` are escaped,
    and every other byte becomes `?`, which keeps free text inside the protocol's string rules."""
    out = ['"']
    for byte in _until_nul(_bytes_of(text)[:max_bytes]):
        if byte in (0x22, 0x5C):
            out.append("\\" + chr(byte))
        elif byte in _PRINTABLE:
            out.append(chr(byte))
        else:
            out.append("?")
    out.append('"')
    return "".join(out)


def _sent(data: bytes, alphabet: frozenset[int]) -> str:
    return "".join(chr(b) if b in alphabet else chr(_REPLACEMENT) for b in _until_nul(data))


def sanitize_ident(text: str | bytes, max_bytes: int) -> str:
    """The form of an id or tag that is sent: bytes outside `[A-Za-z0-9._:/-]` become `_`, at most `max_bytes`."""
    return _sent(_bytes_of(text)[:max_bytes], _ALPHABET)


def ident(text: str | bytes, max_bytes: int) -> str:
    return '"' + sanitize_ident(text, max_bytes) + '"'


def thread_key(text: str | bytes, max_bytes: int) -> str:
    """Like an id, but `#` is allowed (claim-order keys are `t#<n>`)."""
    return '"' + _sent(_bytes_of(text)[:max_bytes], _THREAD_ALPHABET) + '"'


def thread_key_padded(text: str | bytes) -> str:
    """The quoted key followed by spaces up to 64 key bytes, so a line's length does not depend on the key."""
    quoted = thread_key(text, THREAD_KEY_MAX)
    used = len(quoted) - 2
    return quoted + " " * max(THREAD_KEY_MAX - used, 0)


def fnv1a64(data: bytes) -> int:
    value = _FNV_OFFSET
    for byte in data:
        value = ((value ^ byte) * _FNV_PRIME) & _MASK64
    return value


def ident_hash(text: str | bytes, max_bytes: int) -> int:
    """FNV-1a 64 over the form of an id that is sent."""
    return fnv1a64(sanitize_ident(text, max_bytes).encode("ascii"))
