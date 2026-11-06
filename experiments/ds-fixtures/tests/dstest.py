"""Shared helpers for the native self-tests of the DS fixtures. Python 3 standard library only.

These tests run the programs natively (no Hermit). Ports come from DS_PORT_BASE; the programs get them as arguments.
"""
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import traceback

import oracle

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
BIN = os.environ.get("DS_BIN") or os.path.join(ROOT, "build")
PORT_BASE = int(os.environ.get("DS_PORT_BASE", "17100"))
RUNS = int(os.environ.get("DS_RUNS", "20"))
KILLS = int(os.environ.get("DS_KILLS", "100"))
GO_KILLS = int(os.environ.get("DS_GO_KILLS", "40"))
PROGRAM_NAMES = ["dsl", "dskv", "dscli", "dsnode", "dsgo", "dsgonode", "dsrs"]
SERVER_PORT = PORT_BASE + 1
CRASH_PORT = PORT_BASE + 100


def exe(name):
    return os.path.join(BIN, name)


def kv(line):
    """Parse the key=value tokens of a result line into a dict (the leading words are skipped)."""
    out = {}
    for tok in line.split():
        if "=" in tok:
            k, v = tok.split("=", 1)
            out[k] = v
    return out


def check(cond, message):
    if not cond:
        raise AssertionError(message)


def run(args, env=None, timeout=120):
    merged = dict(os.environ)
    merged.pop("HAIRSPRING_PROC_PID", None)
    if env:
        merged.update(env)
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout, env=merged)


class Trial:
    """A fresh trial directory, removed on exit."""

    def __enter__(self):
        self.dir = tempfile.mkdtemp(prefix="dsfx-")
        return self.dir

    def __exit__(self, *exc):
        shutil.rmtree(self.dir, ignore_errors=True)


def port_is_free(port):
    s = socket.socket()
    try:
        s.settimeout(1)
        return s.connect_ex(("127.0.0.1", port)) != 0
    finally:
        s.close()


def process_running(name):
    """True if a process with exactly this name exists. Only the exit status is used: nothing is printed."""
    return subprocess.run(["pgrep", "-x", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0


def check_no_leftovers(ports):
    for name in PROGRAM_NAMES:
        check(not process_running(name), "a %s process is still running" % name)
    for p in ports:
        check(port_is_free(p), "port %d is still accepting connections" % p)


LAUNCH_N0 = re.compile(
    r"^DSN0 v1 role=launch servers=3 clients=2 exits=n1:0,n2:0,n3:0,c1:0,c2:0 stragglers=0 live_after=0 seq=160 "
    r"seq_ok=1 hist=[0-9a-f]{16} ok=1$"
)
SRV_N0 = re.compile(
    r"^DSN0 v1 role=srv node=n[123] mode=(epoll|timeout|block|go|rust) applied=\d+ repl_in=\d+ repl_out=\d+ "
    r"refused=\d+ hb=\d+ suspect=\d+ clock=[0-9a-f]{16} state=[0-9a-f]{16}$"
)
CLI_N0 = re.compile(r"^DSN0 v1 role=cli node=c[12] ops=40 ok=40 fail=0 refused=\d+ hist=[0-9a-f]{16}$")


def run_n0(topo, port_base=None, extra=None, tmp=None):
    base = PORT_BASE if port_base is None else port_base
    args = [exe("dsl"), "--topo=" + topo, "--bin-dir=" + BIN, "--port-base=%d" % base]
    args += extra or []
    if tmp is None:
        with Trial() as t:
            return run(args + ["--tmp=" + t])
    return run(args + ["--tmp=" + tmp])


def check_n0_ok(topo, result, conserve=False, mode=None):
    """One N0 trial: rc 0, exactly six well-formed lines, launcher ok, clients fail=0."""
    check(result.returncode == 0, "%s: rc %d, stdout %r stderr %r" % (topo, result.returncode, result.stdout, result.stderr))
    lines = result.stdout.splitlines()
    check(len(lines) == 6, "%s: expected 6 lines, got %d: %r" % (topo, len(lines), result.stdout))
    srv = [l for l in lines if " role=srv " in l]
    cli = [l for l in lines if " role=cli " in l]
    launch = [l for l in lines if " role=launch " in l]
    check(len(srv) == 3 and len(cli) == 2 and len(launch) == 1, "%s: wrong line mix: %r" % (topo, lines))
    check(all(SRV_N0.match(l) for l in srv), "%s: bad server line in %r" % (topo, srv))
    check(all(CLI_N0.match(l) for l in cli), "%s: bad client line in %r" % (topo, cli))
    check(LAUNCH_N0.match(launch[0]), "%s: bad launcher line %r" % (topo, launch[0]))
    if mode:
        check(all(kv(l)["mode"] == mode for l in srv), "%s: wrong mode in %r" % (topo, srv))
    fields = [kv(l) for l in srv]
    check(sum(int(f["applied"]) for f in fields) == 60, "%s: applied should total 60" % topo)
    if conserve:
        check(
            sum(int(f["repl_in"]) for f in fields) == sum(int(f["repl_out"]) for f in fields) == 60,
            "%s: replication in/out not conserved: %r" % (topo, srv),
        )
    return fields


# ---- crash experiment trial checks ----

START_C0 = re.compile(r"^DSC0 v1 role=node tag=(n1\.i\d+) recovered=(\d+) torn=(\d+)$")
EXIT_C0 = re.compile(r"^DSC0 v1 role=node tag=(n1\.i\d+) exit=quit done=(\d+)$")
KILLED = re.compile(r"^HAIRSPRING_EVENT (\{.*\})$")
LAUNCH_C0 = re.compile(
    r"^DSC0 v1 role=launch kills=(\d+) restarts=(\d+) stragglers=0 live_after=0 seq_ok=1 hist=[0-9a-f]{16} ok=1$"
)


def run_c0(seed, kills, kind="pid", node_bin="dsnode", flags=None, port=None, tmp=None, timeout=120):
    args = [
        exe("dsl"),
        "--crash=%s,%d" % (kind, kills),
        "--bin-dir=" + BIN,
        "--port=%d" % (CRASH_PORT if port is None else port),
        "--seed=%d" % seed,
        "--node-bin=" + node_bin,
    ] + (flags or [])
    if tmp is None:
        with Trial() as t:
            return run(args + ["--tmp=" + t], timeout=timeout)
    return run(args + ["--tmp=" + tmp], timeout=timeout)


def read_bytes(path):
    with open(path, "rb") as f:
        return f.read()


def check_c0_trial(result, tmp, seed, kills, group=False, expect_helper=False):
    """Verify one crash trial against the oracle. Returns (lands, resent_total)."""
    check(result.returncode == 0, "c0: rc %d, stdout %r stderr %r" % (result.returncode, result.stdout, result.stderr))
    lines = result.stdout.splitlines()
    starts = [START_C0.match(l) for l in lines if " role=node " in l and " recovered=" in l]
    exits = [EXIT_C0.match(l) for l in lines if " exit=" in l]
    sups = [kv(l) for l in lines if " role=sup " in l]
    killed = [KILLED.match(l) for l in lines if l.startswith("HAIRSPRING_EVENT")]
    clis = [kv(l) for l in lines if " role=cli " in l]
    launch = [l for l in lines if " role=launch " in l]
    check(all(starts) and len(starts) == kills + 1, "c0: expected %d start lines: %r" % (kills + 1, lines))
    check(len(exits) == 1 and exits[0], "c0: expected one exit line: %r" % lines)
    check(len(sups) == kills and len(killed) == kills and len(clis) == 1 and len(launch) == 1, "c0: wrong line mix: %r" % lines)
    check(len(lines) == len(starts) + 1 + 2 * kills + 1 + 1, "c0: unexpected extra lines: %r" % lines)
    check(LAUNCH_C0.match(launch[0]) and kv(launch[0])["kills"] == str(kills), "c0: bad launcher line %r" % launch)
    check(kv(launch[0])["restarts"] == str(kills), "c0: restarts should equal kills")
    plan = oracle.crash_plan(seed, kills)
    for j, (_, d, _r) in enumerate(plan, start=1):
        tag = "n1.i%d" % (j - 1)
        s = sups[j - 1]
        check(s["kill"] == str(j) and s["victim"] == tag, "c0: sup line %d wrong: %r" % (j, s))
        check(s["wifsig"] == "1" and s["termsig"] == "9", "c0: kill %d was not a SIGKILL: %r" % (j, s))
        check(s["planned_ms"] == str(d), "c0: planned_ms %s differs from the oracle plan %d" % (s["planned_ms"], d))
        check(s["grp_gone"] == ("1" if group else "na"), "c0: grp_gone wrong: %r" % s)
        ev = json.loads(killed[j - 1].group(1))
        check(
            ev == {"v": 1, "kind": "killed", "proc": "_launcher", "target": tag, "signal": 9, "seq": j, "point": "t%d" % d},
            "c0: killed line %d wrong: %r" % (j, ev),
        )
    # Region invariant for every incarnation, killed or not.
    log = read_bytes(os.path.join(tmp, "ds-data", "n1.log"))
    records, torn = oracle.parse_log(log)
    check(torn == 0, "c0: the log has %d torn entries" % torn)
    for inc in range(kills + 1):
        region = oracle.parse_region(read_bytes(os.path.join(tmp, "hairspring-region", "n1.i%d" % inc)))
        check(region[2] == "n1.i%d" % inc, "c0: region tag wrong: %r" % (region,))
        bad = oracle.region_invariant(region, inc, records, killed=inc < kills)
        check(bad == "", "c0: region of n1.i%d breaks %s: %r" % (inc, bad, region))
        if inc == kills:
            check(exits[0].group(2) == str(region[4]), "c0: exit line done= differs from the region")
    written_by_killed = [sum(1 for r in records if r[1] == i and r[2] != oracle.HEARTBEAT_CLIENT) for i in range(kills)]
    cli = clis[0]
    seen = int(cli["resent"]) + int(cli["eof"]) + int(cli["reset"])
    lands = all(w >= 1 for w in written_by_killed) and seen >= 1
    check(cli["ops"] == "60" and cli["ok"] == "60", "c0: client did not complete 60 operations: %r" % cli)
    return lands, int(cli["resent"])


# ---- a tiny runner ----

def run_tests(tests):
    failed = 0
    for fn in tests:
        try:
            fn()
            print("PASS %s" % fn.__name__)
        except Exception:  # noqa: BLE001 - report every failure with its traceback
            failed += 1
            print("FAIL %s" % fn.__name__)
            traceback.print_exc()
    print("%d of %d passed" % (len(tests) - failed, len(tests)))
    return 1 if failed else 0
