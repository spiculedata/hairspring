"""Independent Python oracle for the DS fixtures.

Everything here is written from the written description of the fixtures, not from the C, Go or Rust code, so that
a disagreement between this file and a program shows a bug in one of them. It holds:

  * the FNV-1a 64 digest used for state and history digests,
  * the client operation lists (a pure function of the client id),
  * the crash plan (splitmix64 finaliser, keyed by salt and kill index),
  * a reader for the 4,096-byte report region and the 32-byte node log records.

Nothing here needs the network or a built program.
"""
import struct

MASK = (1 << 64) - 1
FNV_OFFSET = 0xCBF29CE484222325
FNV_PRIME = 0x100000001B3
CRASH_SALT = 0x6372617368  # "crash"
KEYS = 8
N0_OPS = 40
C0_OPS = 60
REGION_BYTES = 4096
RECORD_BYTES = 32
RECORD_MAGIC = 0x44534331
HEARTBEAT_CLIENT = 0xFFFFFFFF


def fnv1a(data: bytes, h: int = FNV_OFFSET) -> int:
    for b in data:
        h = ((h ^ b) * FNV_PRIME) & MASK
    return h


def state_digest(table: dict) -> int:
    """Digest of a key table: for each present key index in order, the text 'k<d>=<value>;'."""
    text = "".join("k%d=%d;" % (k, table[k]) for k in sorted(table))
    return fnv1a(text.encode())


def op_list(client_id: int, reconnect: bool, servers: int = 3):
    """The operations of a client as (index, 'put'|'get', key, value-or-None, server)."""
    count = C0_OPS if reconnect else N0_OPS
    ops = []
    for i in range(count):
        is_put = True if reconnect else (i % 4 != 3)
        key = (i * (2 * client_id + 1) + client_id) % KEYS
        server = 0 if reconnect else (i + client_id) % servers
        value = client_id * 1000 + i if is_put else None
        ops.append((i, "put" if is_put else "get", key, value, server))
    return ops


def op_lines(client_id: int, reconnect: bool, servers: int = 3):
    out = []
    for i, kind, key, value, server in op_list(client_id, reconnect, servers):
        out.append("%d %s k%d %s s%d" % (i, kind, key, "-" if value is None else value, server))
    return out


def mix(x: int) -> int:
    z = x & MASK
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
    return z ^ (z >> 31)


def draw(seed: int, salt: int, j: int) -> int:
    return mix((seed & MASK) ^ mix(salt ^ j))


def crash_plan(seed: int, kills: int):
    """List of (kill index j, d_j, r_j) for j = 1..kills."""
    plan = []
    d_prev = r_prev = 0
    for j in range(1, kills + 1):
        r = 20 + draw(seed, CRASH_SALT, 2 * j - 1) % 61
        if j == 1:
            d = 40 + draw(seed, CRASH_SALT, 0) % 241
        else:
            d = d_prev + r_prev + 30 + draw(seed, CRASH_SALT, 2) % 121
        plan.append((j, d, r))
        d_prev, r_prev = d, r
    return plan


def plan_lines(seed: int, kills: int):
    return ["kill=%d planned_ms=%d restart_ms=%d" % p for p in crash_plan(seed, kills)]


def parse_region(blob: bytes):
    """Return (ok, reason, tag, started, done, acked) for a region file's bytes."""
    if len(blob) != REGION_BYTES:
        return False, "size", None, 0, 0, 0
    magic, version, header_bytes, tag = struct.unpack_from("<8sII48s", blob, 0)
    if magic != b"HSRGNv1\0":
        return False, "magic", None, 0, 0, 0
    if version != 1:
        return False, "version", None, 0, 0, 0
    if header_bytes != 64:
        return False, "header", None, 0, 0, 0
    started, done, acked = struct.unpack_from("<QQQ", blob, 64)
    return True, "", tag.rstrip(b"\0").decode(), started, done, acked


def parse_log(blob: bytes):
    """Return (records, torn). A record is (magic, incarnation, client, op, key, value, reserved)."""
    records = []
    torn = 0
    full = len(blob) // RECORD_BYTES
    for n in range(full):
        rec = struct.unpack_from("<IIIIIQI", blob, n * RECORD_BYTES)
        if rec[0] == RECORD_MAGIC:
            records.append(rec)
        else:
            torn += 1
    if len(blob) % RECORD_BYTES:
        torn += 1
    return records, torn


def make_record(incarnation: int, client: int, op: int, key: int, value: int) -> bytes:
    return struct.pack("<IIIIIQI", RECORD_MAGIC, incarnation, client, op, key, value, 0)


def region_invariant(region, incarnation: int, records, killed: bool):
    """The crash experiment region checks for one incarnation. Returns '' when they hold, else the rule that failed."""
    ok, reason, _tag, started, done, acked = region
    if not ok:
        return reason
    written = sum(1 for r in records if r[1] == incarnation)
    if not (done <= written <= started):
        return "written-between-done-and-started"
    if started - done > 1:
        return "started-minus-done"
    if acked > done:
        return "acked-above-done"
    if done - acked > 1:
        return "done-minus-acked"
    if not killed and done != written:
        return "done-is-written"
    return ""


if __name__ == "__main__":
    import sys

    cmd = sys.argv[1]
    if cmd == "ops":
        for line in op_lines(int(sys.argv[2]), sys.argv[3] == "reconnect"):
            print(line)
    elif cmd == "plan":
        for line in plan_lines(int(sys.argv[2]), int(sys.argv[3])):
            print(line)
    elif cmd == "fnv":
        print("%016x" % fnv1a(sys.argv[2].encode()))
    elif cmd == "state":
        print("%016x" % state_digest({int(a.split("=")[0]): int(a.split("=")[1]) for a in sys.argv[2:]}))
    else:
        sys.exit("usage: oracle.py ops <client> normal|reconnect | plan <seed> <kills> | fnv <text> | state k=v ...")
