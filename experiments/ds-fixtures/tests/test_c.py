"""Native self-test of the C fixtures (dsl, dskv, dscli, dsnode). Not run under Hermit.

Covers: operation-list and crash-plan golden files against the independent oracle, protocol behaviour of dskv and
dsnode against a scripted peer, the dscli history file against a scripted server, recovery of the node log,
every N0 topology and every C0 topology RUNS times, the launcher's early-exit and straggler handling, and (Linux with
strace) the syscall adjacency the kill supervisor promises.
"""
import os
import re
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import threading
import time

import oracle
from dstest import (
    CLI_C0,
    LAUNCH_C0,
    SUP_C0,
    parse_events,
    BIN,
    CRASH_PORT,
    HERE,
    PORT_BASE,
    ROOT,
    RUNS,
    Trial,
    check,
    check_c0_trial,
    check_n0_ok,
    check_no_leftovers,
    exe,
    kv,
    process_running,
    read_bytes,
    run,
    run_c0,
    run_n0,
    run_tests,
)

GOLDEN = os.path.join(HERE, "golden")
PLAN_SEEDS = [620001] + list(range(620101, 620111)) + [400001, 491001, 491002, 491003, 1, 2, 3, 18446744073709551615, 610001]


def golden(name):
    with open(os.path.join(GOLDEN, name)) as f:
        return f.read().splitlines()


def test_operation_lists_match_golden_and_oracle():
    for cid in (1, 2):
        for mode, flag in (("normal", []), ("reconnect", ["--reconnect"])):
            got = run([exe("dscli"), "--node=c%d" % cid, "--print-ops"] + flag).stdout.splitlines()
            check(got == golden("ops-c%d-%s.txt" % (cid, mode)), "dscli ops differ from golden for c%d %s" % (cid, mode))
            check(got == oracle.op_lines(cid, mode == "reconnect"), "dscli ops differ from the oracle for c%d %s" % (cid, mode))


def test_crash_plan_matches_golden_and_oracle():
    expected = golden("plan.txt")
    got = []
    for seed in PLAN_SEEDS:
        out = run([exe("dsl"), "--print-plan", "--seed=%d" % seed, "--kills=2"]).stdout.splitlines()
        check(out == oracle.plan_lines(seed, 2), "plan for seed %d differs from the oracle: %r" % (seed, out))
        got += ["seed=%d %s" % (seed, l) for l in out]
        one = run([exe("dsl"), "--print-plan", "--seed=%d" % seed, "--kills=1"]).stdout.splitlines()
        check(one == out[:1], "the one-kill plan must be the first line of the two-kill plan (seed %d)" % seed)
    check(got == expected, "plan differs from the golden file")
    plans = {tuple(oracle.crash_plan(s, 2)) for s in PLAN_SEEDS}
    check(len(plans) == len(PLAN_SEEDS), "the 20 seeds should give 20 different plans")


def test_the_seed_comes_only_from_the_seed_argument():
    r = run([exe("dsl"), "--print-plan", "--kills=1", "--seed=620001"], env={"HAIRSPRING_SEED": "1"})
    check(r.stdout.splitlines() == oracle.plan_lines(620001, 1), "the --seed argument must win")
    # An environment variable is never a seed: without --seed the launcher refuses, whatever the environment holds.
    r = run([exe("dsl"), "--print-plan", "--kills=1"], env={"HAIRSPRING_SEED": "620001"})
    check(r.returncode == 2 and r.stdout == "", "HAIRSPRING_SEED must not be read: rc %d %r" % (r.returncode, r.stdout))
    with Trial() as tmp:
        r = run([exe("dsl"), "--crash=pid,1", "--bin-dir=" + BIN, "--tmp=" + tmp, "--port=%d" % CRASH_PORT], env={"HAIRSPRING_SEED": "620001"})
        check(r.returncode == 2 and r.stdout == "", "a crash trial without --seed must be refused: rc %d" % r.returncode)
    check_no_leftovers([CRASH_PORT])


def _listener(port):
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", port))
    s.listen(4)
    s.settimeout(10)
    return s


def _read_lines(conn, count):
    buf = b""
    conn.settimeout(10)
    while buf.count(b"\n") < count:
        data = conn.recv(4096)
        if not data:
            break
        buf += data
    return buf.decode().splitlines()


def test_dskv_protocol_against_a_scripted_peer():
    port, peer_port = PORT_BASE + 20, PORT_BASE + 21
    peer = _listener(peer_port)
    server = subprocess.Popen(
        [exe("dskv"), "--mode=epoll", "--node=n1", "--port=%d" % port, "--peer-port=%d" % peer_port],
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        succ, _ = peer.accept()
        check(_read_lines(succ, 1) == ["HELLO n1"], "the successor should first receive HELLO n1")
        c = socket.create_connection(("127.0.0.1", port), timeout=10)
        c.sendall(b"PUT k3 7\nGET k3\nGET k4\nBOGUS\nPUT k9 1\nREPL k5 9\n")
        replies = _read_lines(c, 5)
        check(replies == ["OK", "VAL 7", "NIL", "ERR", "ERR"], "unexpected replies %r" % replies)
        check(_read_lines(succ, 1) == ["REPL k3 7"], "PUT should be replicated to the successor")
        c.sendall(b"QUIT\n")
        out, _ = server.communicate(timeout=20)
    finally:
        if server.poll() is None:
            server.kill()
            server.wait()
        peer.close()
    check(server.returncode == 0, "dskv exit status %d" % server.returncode)
    f = kv(out.strip())
    digest = "%016x" % oracle.state_digest({3: 7, 5: 9})
    check(
        out.count("\n") == 1 and f["applied"] == "1" and f["repl_in"] == "1" and f["repl_out"] == "1" and f["state"] == digest,
        "unexpected result line %r (want state %s)" % (out, digest),
    )


def test_dskv_retries_a_connect_made_before_the_peer_listens():
    port, peer_port = PORT_BASE + 22, PORT_BASE + 23
    server = subprocess.Popen(
        [exe("dskv"), "--mode=epoll", "--node=n1", "--port=%d" % port, "--peer-port=%d" % peer_port],
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        time.sleep(0.3)
        peer = _listener(peer_port)
        succ, _ = peer.accept()
        _read_lines(succ, 1)
        c = socket.create_connection(("127.0.0.1", port), timeout=10)
        c.sendall(b"QUIT\n")
        out, _ = server.communicate(timeout=20)
        peer.close()
    finally:
        if server.poll() is None:
            server.kill()
            server.wait()
    check(int(kv(out.strip())["refused"]) >= 1, "the server should have counted a refused connect: %r" % out)


def test_dskv_block_mode_serves_one_connection_at_a_time():
    port = PORT_BASE + 24
    server = subprocess.Popen([exe("dskv"), "--mode=block", "--node=n1", "--port=%d" % port], stdout=subprocess.PIPE, text=True)
    try:
        for attempt in range(100):
            try:
                c = socket.create_connection(("127.0.0.1", port), timeout=2)
                break
            except OSError:
                time.sleep(0.05)
        c.sendall(b"PUT k1 5\n")
        check(_read_lines(c, 1) == ["OK"], "block mode should reply OK")
        c.close()
        c = socket.create_connection(("127.0.0.1", port), timeout=10)
        c.sendall(b"GET k1\nQUIT\n")
        check(_read_lines(c, 1) == ["VAL 5"], "block mode should keep its table across connections")
        out, _ = server.communicate(timeout=20)
    finally:
        if server.poll() is None:
            server.kill()
            server.wait()
    f = kv(out.strip())
    check(f["mode"] == "block" and f["repl_out"] == "0" and f["state"] == "%016x" % oracle.state_digest({1: 5}), "bad line %r" % out)


def _fake_kv_server(port, ready, stop):
    """A scripted server for dscli: PUT -> OK, GET -> NIL or VAL, until stop is set."""
    s = _listener(port)
    ready.set()
    conn, _ = s.accept()
    table = {}
    buf = b""
    conn.settimeout(10)
    while not stop.is_set():
        try:
            data = conn.recv(4096)
        except OSError:
            break
        if not data:
            break
        buf += data
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            parts = line.decode().split()
            if parts[0] == "PUT":
                table[parts[1]] = parts[2]
                conn.sendall(b"OK\n")
            else:
                conn.sendall(("VAL %s\n" % table[parts[1]]).encode() if parts[1] in table else b"NIL\n")
    conn.close()
    s.close()


def test_dscli_history_against_a_scripted_server():
    port = PORT_BASE + 25
    ready, stop = threading.Event(), threading.Event()
    t = threading.Thread(target=_fake_kv_server, args=(port, ready, stop))
    t.start()
    try:
        ready.wait(10)
        with Trial() as tmp:
            os.mkdir(os.path.join(tmp, "ds-hist"))
            with open(os.path.join(tmp, "ds-seq"), "wb") as f:
                f.write(struct.pack("<Q", 1000) + b"\0" * 4088)  # a mocked counter that starts at 1000
            r = run([exe("dscli"), "--node=c1", "--servers=%d" % port, "--tmp=" + tmp])
            check(r.returncode == 0, "dscli rc %d" % r.returncode)
            blob = read_bytes(os.path.join(tmp, "ds-hist", "c1"))
    finally:
        stop.set()
        t.join(20)
    lines = blob.decode().splitlines()
    check(len(lines) == 80, "40 operations give 80 records, got %d" % len(lines))
    check([int(l.split()[0]) for l in lines] == list(range(1001, 1081)), "seq values must be contiguous from the counter")
    fmt = re.compile(r"^\d+ c1 (inv|ret) (put|get) k[0-7] (\d+|-) (-|OK|NIL|\d+)$")
    check(all(fmt.match(l) for l in lines), "a history line is malformed")
    check(lines[0] == "1001 c1 inv put k1 1000 -" and lines[1] == "1002 c1 ret put k1 1000 OK", "first records wrong: %r" % lines[:2])
    f = kv(r.stdout.strip())
    check(f["ok"] == "40" and f["fail"] == "0" and f["hist"] == "%016x" % oracle.fnv1a(blob), "bad result line %r" % r.stdout)


def test_dscli_retries_connect_before_listen():
    port = PORT_BASE + 26
    with Trial() as tmp:
        os.mkdir(os.path.join(tmp, "ds-hist"))
        with open(os.path.join(tmp, "ds-seq"), "wb") as f:
            f.write(b"\0" * 4096)
        cli = subprocess.Popen([exe("dscli"), "--node=c2", "--servers=%d" % port, "--tmp=" + tmp], stdout=subprocess.PIPE, text=True)
        time.sleep(0.3)
        ready, stop = threading.Event(), threading.Event()
        t = threading.Thread(target=_fake_kv_server, args=(port, ready, stop))
        t.start()
        try:
            out, _ = cli.communicate(timeout=60)
        finally:
            stop.set()
            t.join(20)
            if cli.poll() is None:
                cli.kill()
                cli.wait()
    f = kv(out.strip())
    check(int(f["refused"]) >= 1 and f["ok"] == "40", "the client should have retried a refused connect: %r" % out)


def _write_log(path, records, tail=b""):
    with open(path, "wb") as f:
        for i in range(records):
            f.write(oracle.make_record(0, 1, i % 1000, i % 8, 1000 + i))
        f.write(tail)


def _recover(tmp, tag="n1.i3"):
    return run([exe("dsnode"), "--node=n1", "--tmp=" + tmp, "--recover-only"], env={"HAIRSPRING_PROC": tag})


def test_dsnode_recovery_counts_records_and_torn_tails():
    cases = [
        (0, b"", "recovered=0 torn=0", 0),
        (1, b"", "recovered=1 torn=0", 32),
        (1000, b"", "recovered=1000 torn=0", 32000),
        (5, b"\x01" * 7, "recovered=5 torn=1", 160),
        (4, b"\0" * 32, "recovered=4 torn=1", 160),
    ]
    for count, tail, expect, size_after in cases:
        with Trial() as tmp:
            os.mkdir(os.path.join(tmp, "ds-data"))
            log = os.path.join(tmp, "ds-data", "n1.log")
            _write_log(log, count, tail)
            r = _recover(tmp)
            check(r.returncode == 0, "recovery rc %d: %r" % (r.returncode, r.stderr))
            check(r.stdout == "DSC0 v1 role=node tag=n1.i3 %s\n" % expect, "recovery line %r, want %s" % (r.stdout, expect))
            check(os.path.getsize(log) == size_after, "log size %d after recovery, want %d" % (os.path.getsize(log), size_after))


def test_dsnode_serves_and_leaves_an_intact_region():
    port = PORT_BASE + 27
    with Trial() as tmp:
        for d in ("ds-data", "hairspring-region"):
            os.mkdir(os.path.join(tmp, d))
        node = subprocess.Popen(
            [exe("dsnode"), "--node=n1", "--port=%d" % port, "--tmp=" + tmp],
            stdout=subprocess.PIPE,
            text=True,
            env=dict(os.environ, HAIRSPRING_PROC="n1.i0"),
        )
        try:
            for _ in range(100):
                try:
                    c = socket.create_connection(("127.0.0.1", port), timeout=2)
                    break
                except OSError:
                    time.sleep(0.05)
            c.sendall(b"PUT k2 2003\nPUT k2 2004\nGET k2\nGET k7\n")
            check(_read_lines(c, 4) == ["OK", "OK", "VAL 2004", "NIL"], "unexpected node replies")
            c.sendall(b"QUIT\n")
            out, _ = node.communicate(timeout=20)
        finally:
            if node.poll() is None:
                node.kill()
                node.wait()
        check(out.splitlines()[-1] == "DSC0 v1 role=node tag=n1.i0 exit=quit done=2", "bad exit line %r" % out)
        records, torn = oracle.parse_log(read_bytes(os.path.join(tmp, "ds-data", "n1.log")))
        check(torn == 0 and [(r[2], r[3], r[4], r[5]) for r in records] == [(2, 3, 2, 2003), (2, 4, 2, 2004)], "bad log %r" % records)
        region = oracle.parse_region(read_bytes(os.path.join(tmp, "hairspring-region", "n1.i0")))
        check(region == (True, "", "n1.i0", 2, 2, 2), "bad region %r" % (region,))


def _repeat(topo, runs, **kw):
    for i in range(runs):
        r = run_n0(topo, **kw)
        yield r
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2])


def _topology_test(topo, mode, conserve):
    def test():
        for r in _repeat(topo, RUNS):
            check_n0_ok(topo, r, conserve=conserve, mode=mode)

    test.__name__ = "test_topology_%s_%d_runs_all_ok" % (topo, RUNS)
    return test


def test_topologies_produce_different_application_results_across_runs():
    # Both clients write the same keys, so the final state can differ across host schedules. This is a native check
    # that the workload exposes interleavings (it can fail: one state in 60 runs fails it), not a claim about any
    # other environment.
    states = set()
    for _ in range(60):
        r = run_n0("c3e")
        states.add(tuple(kv(l)["state"] for l in sorted(r.stdout.splitlines()) if " role=srv " in l))
        if len(states) >= 2:
            break
    check(len(states) >= 2, "60 native runs of c3e all ended in the same state: the workload hides interleavings")
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2])


def test_early_exit_of_a_server_gives_ok0_and_rc3():
    r = run_n0("c3e", extra=["--server-arg=n2:--test-exit-early"])
    check(r.returncode == 3, "rc should be 3, got %d" % r.returncode)
    launch = [l for l in r.stdout.splitlines() if " role=launch " in l][0]
    f = kv(launch)
    check(f["ok"] == "0" and f["live_after"] == "0" and "n2:1" in f["exits"], "bad launcher line %r" % launch)
    check("c1:3" in f["exits"] or "c2:3" in f["exits"], "a client with failed operations must exit 3: %r" % f["exits"])
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2])


def test_a_server_that_ignores_quit_is_killed_and_counted():
    r = run_n0("c3e", extra=["--server-arg=n3:--test-ignore-quit"])
    check(r.returncode == 3, "rc should be 3, got %d" % r.returncode)
    f = kv([l for l in r.stdout.splitlines() if " role=launch " in l][0])
    check(f["stragglers"] == "1" and f["ok"] == "0" and "n3:sig9" in f["exits"] and f["live_after"] == "0", "bad launcher line %r" % f)
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2])


def _crash_test(name, kills, kind="pid", flags=None, seeds=None):
    flags = flags or []

    def test():
        for i in range(RUNS):
            seed = 620001 + i if seeds is None else seeds[i % len(seeds)]
            with Trial() as tmp:
                r = run_c0(seed, kills, kind=kind, flags=flags, tmp=tmp)
                check_c0_trial(r, tmp, seed, kills, group=kind == "pgid")
        check_no_leftovers([CRASH_PORT])

    test.__name__ = "test_crash_%s_%d_runs_all_ok" % (name, RUNS)
    return test


def test_crash_kills_land_in_the_workload():
    lands = 0
    seeds = list(range(620101, 620111))
    for seed in seeds:
        with Trial() as tmp:
            r = run_c0(seed, 1, tmp=tmp)
            landed, _ = check_c0_trial(r, tmp, seed, 1)
            lands += landed
    check(lands >= 8, "only %d of %d kills landed mid-workload" % (lands, len(seeds)))
    print("INFO natively %d of %d kills landed mid-workload" % (lands, len(seeds)))


def test_crash_process_group_kill_removes_the_helper():
    with Trial() as tmp:
        r = run_c0(620001, 1, kind="pgid", flags=["--helper"], tmp=tmp)
        check_c0_trial(r, tmp, 620001, 1, group=True)
    check(not process_running("dsnode"), "a dsnode is left")


def test_crash_write_then_kill_diagnostic_trial():
    with Trial() as tmp:
        r = run_c0(620001, 1, flags=["--pause", "--raw-write-kill"], tmp=tmp)
        check(r.returncode == 0, "rc %d: %r" % (r.returncode, r.stdout))
        lines = r.stdout.splitlines()
        check(len(lines) == 6, "expected 6 lines, got %r" % lines)
        check(lines[0] == "DSC0 v1 role=node tag=n1.i0 recovered=0 torn=0", "bad first line %r" % lines)
        check(lines[1] == "DSC0 v1 role=sup pre-kill" and len(lines[1]) + 1 == 26, "bad pre-kill line %r" % lines[1])
        _l, killed, exited, sups, launch = _split(r.stdout)
        check(killed == [{"v": 1, "kind": "killed", "proc": "_launcher", "target": "n1.i0", "signal": 9, "seq": 1, "point": "t50"}], "killed %r" % killed)
        check(exited == [{"v": 1, "kind": "exited", "proc": "_launcher", "target": "n1.i0", "status": 9, "cause": "planned_kill"}], "exited %r" % exited)
        check(SUP_C0.match(lines[4]) and sups[0]["pre_dead"] == "0" and sups[0]["region_seen"] == "1" and sups[0]["planned_ms"] == "50", "sup %r" % lines[4])
        check(LAUNCH_C0.match(lines[5]) and launch["kills"] == "1" and launch["restarts"] == "0", "launcher %r" % lines[5])
    check_no_leftovers([CRASH_PORT])


def _erring_server(port, ready, stop):
    """A scripted server that answers every request with ERR."""
    s = _listener(port)
    ready.set()
    conn, _ = s.accept()
    conn.settimeout(10)
    buf = b""
    while not stop.is_set():
        try:
            data = conn.recv(4096)
        except OSError:
            break
        if not data:
            break
        buf += data
        while b"\n" in buf:
            _, buf = buf.split(b"\n", 1)
            conn.sendall(b"ERR\n")
    conn.close()
    s.close()


def _client_against_erring_server(port, extra):
    ready, stop = threading.Event(), threading.Event()
    t = threading.Thread(target=_erring_server, args=(port, ready, stop))
    t.start()
    try:
        ready.wait(10)
        with Trial() as tmp:
            os.mkdir(os.path.join(tmp, "ds-hist"))
            with open(os.path.join(tmp, "ds-seq"), "wb") as f:
                f.write(b"\0" * 4096)
            return run([exe("dscli"), "--node=c1", "--servers=%d" % port, "--tmp=" + tmp] + extra)
    finally:
        stop.set()
        t.join(20)


def test_a_client_with_failed_operations_exits_3_in_both_modes():
    r = _client_against_erring_server(PORT_BASE + 31, [])
    f = kv(r.stdout.strip())
    check(r.returncode == 3 and f["ok"] == "0" and f["fail"] == "40", "ring client: rc %d %r" % (r.returncode, r.stdout))
    r = _client_against_erring_server(PORT_BASE + 32, ["--reconnect"])
    check(CLI_C0.match(r.stdout.strip()), "crash client line has the wrong shape: %r" % r.stdout)
    f = kv(r.stdout.strip())
    check(r.returncode == 3 and f["ok"] == "0" and f["fail"] == "60", "crash client: rc %d %r" % (r.returncode, r.stdout))


def _build_c(tmp, source, flags, name):
    out = os.path.join(tmp, name)
    cc = os.environ.get("CC", "cc")
    b = subprocess.run([cc, "-std=gnu11", "-O1", "-pthread"] + flags + ["-o", out, source], capture_output=True, text=True)
    check(b.returncode == 0, "build of %s failed: %s" % (name, b.stderr[-300:]))
    return out


def test_pure_helpers_and_the_output_and_path_guards():
    with Trial() as tmp:
        unit = _build_c(tmp, os.path.join(ROOT, "tests", "unit_c.c"), ["-Wall", "-Wextra"], "unit_c")
        r = run([unit, "logic"])
        check(r.returncode == 0, "helper checks failed: %s" % r.stderr)
        # An output line of 4,096 bytes with its newline is written whole; one byte more is not written at all.
        r = run([unit, "emit", "4096"])
        check(r.returncode == 0 and len(r.stdout) == 4096 and r.stdout.endswith("\n"), "a 4096-byte line must be written whole")
        for n in ("4097", "8192"):
            r = run([unit, "emit", n])
            check(r.returncode == 2 and r.stdout == "" and "too long" in r.stderr, "a %s-byte line must be refused: rc %d" % (n, r.returncode))
        # A path that does not fit its buffer is refused, never shortened (63 characters fit a 64-byte buffer).
        r = run([unit, "fmt", "63"])
        check(r.returncode == 0 and r.stdout == "ok\n", "a path that fits must be accepted")
        for n in ("64", "100", "8192"):
            r = run([unit, "fmt", n])
            check(r.returncode == 2 and r.stdout == "" and "too long" in r.stderr, "a %s-character path must be refused" % n)


def _stop_leftover_nodes():
    """Test cleanup: end any dsnode left by a stub trial, by pid, never by command line."""
    out = subprocess.run(["pgrep", "-x", "dsnode"], capture_output=True, text=True).stdout.split()
    for pid in out:
        try:
            os.kill(int(pid), signal.SIGKILL)
        except OSError:
            pass
    time.sleep(0.2)


def _trial_to_file(args, tmp):
    """Run a launcher with stdout to a file (a stub trial can leave a node that holds a pipe open)."""
    out = os.path.join(tmp, "stdout.txt")
    with open(out, "w") as f:
        rc = subprocess.run(args, stdout=f, stderr=subprocess.PIPE, text=True, timeout=120).returncode
    return rc, open(out).read()


def _crash_args(dsl, trial, bin_dir=None, seed=620001, kills=1, flags=None):
    return [dsl, "--crash=pid,%d" % kills, "--bin-dir=" + (bin_dir or BIN), "--port=%d" % CRASH_PORT, "--seed=%d" % seed, "--pace-ms=10", "--tmp=" + trial] + (flags or [])


def _split(stdout):
    lines = stdout.splitlines()
    events = parse_events(lines)
    return (
        lines,
        [e for e in events if e["kind"] == "killed"],
        [e for e in events if e["kind"] == "exited"],
        [kv(l) for l in lines if " role=sup kill=" in l],
        kv([l for l in lines if " role=launch " in l][0]),
    )


def test_a_victim_that_died_before_the_kill_is_not_reported_as_killed():
    with Trial() as tmp:
        p = subprocess.Popen(_crash_args(exe("dsl"), os.path.join(tmp, "t")), stdout=subprocess.PIPE, text=True)
        try:
            for _ in range(400):  # end the first incarnation early with QUIT, well before the 168 ms kill point
                try:
                    c = socket.create_connection(("127.0.0.1", CRASH_PORT), timeout=2)
                    c.sendall(b"QUIT\n")
                    c.close()
                    break
                except OSError:
                    time.sleep(0.005)
            out, _ = p.communicate(timeout=60)
        finally:
            if p.poll() is None:
                p.kill()
                p.wait()
    lines, killed, exited, sups, launch = _split(out)
    check(p.returncode == 3 and launch["ok"] == "0", "a pre-dead victim must give ok=0, rc 3: rc %d %r" % (p.returncode, launch))
    check(sups[0]["pre_dead"] == "1" and killed == [], "no killed line for a pre-dead victim: %r %r" % (sups, killed))
    first = [e for e in exited if e["target"] == "n1.i0"]
    check(len(first) == 1 and first[0]["cause"] == "exit" and first[0]["status"] == 0, "cause must be exit, never planned_kill: %r" % first)
    check(int(launch["unexpected"]) >= 1 and launch["restarts"] == "1", "bad launcher line %r" % launch)
    check_no_leftovers([CRASH_PORT])


def test_an_unexpected_death_of_the_final_incarnation_is_counted():
    with Trial() as tmp:
        p = subprocess.Popen(_crash_args(exe("dsl"), os.path.join(tmp, "t")), stdout=subprocess.PIPE, text=True)
        head = []
        try:
            for line in p.stdout:
                head.append(line)
                if '"kind":"killed"' in line:
                    break
            pid = None
            for _ in range(600):
                found = subprocess.run(["pgrep", "-x", "dsnode"], capture_output=True, text=True).stdout.split()
                if found:
                    pid = int(found[0])
                    break
                time.sleep(0.005)
            check(pid is not None, "the second incarnation never appeared")
            os.kill(pid, signal.SIGKILL)
            rest = p.stdout.read()
            p.wait(timeout=60)
        finally:
            if p.poll() is None:
                p.kill()
                p.wait()
    lines, killed, exited, sups, launch = _split("".join(head) + rest)
    final = [e for e in exited if e["target"] == "n1.i1"]
    check(len(final) == 1 and final[0]["cause"] == "signal" and final[0]["status"] == 9, "final incarnation: %r" % final)
    check(p.returncode == 3 and launch["ok"] == "0" and int(launch["unexpected"]) >= 1, "bad launcher line %r rc %d" % (launch, p.returncode))
    _stop_leftover_nodes()
    check_no_leftovers([CRASH_PORT])


def _fake_bin_dir(tmp, scripts):
    d = os.path.join(tmp, "fakebin")
    os.mkdir(d)
    for name in ("dsl", "dscli", "dsnode", "dskv"):
        os.symlink(exe(name), os.path.join(d, name))
    for name, body in scripts.items():
        path = os.path.join(d, name)
        if os.path.lexists(path):
            os.unlink(path)
        with open(path, "w") as f:
            f.write("#!/bin/sh\n" + body)
        os.chmod(path, 0o755)
    return d


def test_a_client_that_exits_3_makes_the_trial_not_ok():
    with Trial() as tmp:
        fake = _fake_bin_dir(tmp, {"dscli": '%s "$@"\nexit 3\n' % exe("dscli")})
        r = run(_crash_args(exe("dsl"), os.path.join(tmp, "t"), bin_dir=fake))
    lines, killed, exited, sups, launch = _split(r.stdout)
    client = [e for e in exited if e["target"] == "c1"]
    check(len(client) == 1 and client[0]["status"] == 768 and client[0]["cause"] == "exit", "client exited line %r" % client)
    check(r.returncode == 3 and launch["ok"] == "0" and int(launch["unexpected"]) >= 1, "bad launcher line %r" % launch)
    check_no_leftovers([CRASH_PORT])


def test_the_write_then_kill_trial_fails_when_the_node_never_started():
    with Trial() as tmp:
        fake = _fake_bin_dir(tmp, {"dsnode": "exec sleep 30\n"})
        r = run_c0(620001, 1, flags=["--pause", "--raw-write-kill"], tmp=os.path.join(tmp, "t"), bin_dir=fake)
    lines, killed, exited, sups, launch = _split(r.stdout)
    check(sups[0]["region_seen"] == "0", "region_seen must be 0: %r" % sups)
    check(r.returncode == 3 and launch["ok"] == "0", "the trial must not be ok: rc %d %r" % (r.returncode, launch))
    check_no_leftovers([CRASH_PORT])


def _stub_trial(mode, tmp):
    dsl = _build_c(tmp, os.path.join(ROOT, "c", "dsl.c"), ["-DDS_STUB_WAITPID=%d" % mode], "dsl-stub%d" % mode)
    rc, out = _trial_to_file(_crash_args(dsl, os.path.join(tmp, "t")), tmp)
    _stop_leftover_nodes()
    return rc, out


def test_an_alive_check_error_is_not_alive_and_never_signals_the_pid():
    with Trial() as tmp:
        rc, out = _stub_trial(1, tmp)
    lines, killed, exited, sups, launch = _split(out)
    first = [e for e in exited if e["target"] == "n1.i0"]
    check(sups[0]["pre_dead"] == "1" and killed == [], "pre_dead=1 and no killed line: %r %r" % (sups, killed))
    check(len(first) == 1 and first[0]["status"] == -1 and first[0]["cause"] == "unreaped", "exited line %r" % first)
    check(rc == 3 and launch["ok"] == "0" and int(launch["unexpected"]) >= 1, "bad launcher line %r rc %d" % (launch, rc))
    check_no_leftovers([CRASH_PORT])


def test_an_unreaped_victim_gets_no_killed_line_and_stops_the_plan():
    with Trial() as tmp:
        rc, out = _stub_trial(2, tmp)
    lines, killed, exited, sups, launch = _split(out)
    check(killed == [], "an unreaped victim must not get a killed line: %r" % killed)
    check(sups[0]["pre_dead"] == "0" and sups[0]["wifsig"] == "0", "sup line %r" % sups)
    first = [e for e in exited if e["target"] == "n1.i0"]
    check(len(first) == 1 and first[0]["status"] == -1 and first[0]["cause"] == "unreaped", "exited line %r" % first)
    starts = [l for l in lines if " recovered=" in l]
    check(len(starts) == 1 and launch["restarts"] == "0", "the plan must stop: no restart (%r, %r)" % (starts, launch))
    check(rc == 3 and launch["ok"] == "0" and int(launch["stragglers"]) >= 1, "the victim must be a straggler: %r rc %d" % (launch, rc))
    check_no_leftovers([CRASH_PORT])


STRACE = shutil.which("strace")
KILL_CALL = re.compile(r"kill\(\d+, SIGKILL")  # the call may print as "kill(N, SIGKILL <unfinished ...>"


def _strace_run(flags):
    """Trace with -ff (one file per process) so that the launcher's calls never interleave with another process's.
    Returns the launcher's calls in order, signal and exit notes removed."""
    with Trial() as tmp:
        prefix = os.path.join(tmp, "trace")
        trial = os.path.join(tmp, "t")
        os.mkdir(trial)
        args = [STRACE, "-ff", "-o", prefix, exe("dsl"), "--crash=pid,1", "--bin-dir=" + BIN, "--port=%d" % CRASH_PORT, "--seed=620001", "--tmp=" + trial] + flags
        r = run(args)
        check(r.returncode == 0, "strace run rc %d: %r" % (r.returncode, r.stderr[-300:]))
        launcher = [n for n in os.listdir(tmp) if n.startswith("trace.")]
        found = []
        for name in launcher:
            with open(os.path.join(tmp, name), errors="replace") as f:
                lines = f.read().splitlines()
            if lines and lines[0].startswith("execve(") and "/dsl\"" in lines[0]:
                found.append(lines)
        check(len(found) == 1, "expected exactly one launcher trace, found %d of %d files" % (len(found), len(launcher)))
        return _launcher_calls(found[0])


def _launcher_calls(lines):
    """The launcher's own calls from one per-process strace file."""
    return [c for c in lines if not c.startswith(("---", "+++", "<..."))]


def test_strace_raw_write_is_immediately_followed_by_kill():
    if STRACE is None:
        check(os.environ.get("DS_REQUIRE_STRACE") != "1", "strace is required but missing")
        print("SKIP strace not available")
        return
    check_write_then_kill(_strace_run(["--pause", "--raw-write-kill"]))


ALIVE_CHECK = re.compile(r"wait4\(\d+, .*WNOHANG.*\) += 0")  # waitpid(pid, &st, WNOHANG) answering "still alive"


def check_write_then_kill(calls):
    """Alive check, one write, kill: nothing else in between."""
    idx = [i for i, c in enumerate(calls) if c.startswith('write(1, "DSC0 v1 role=sup pre-kill\\n", 26)')]
    check(len(idx) == 1, "expected exactly one pre-kill write in the launcher's calls")
    i = idx[0]
    check(i > 0 and ALIVE_CHECK.match(calls[i - 1]), "the call before the write is not the alive check")
    check(i + 1 < len(calls) and KILL_CALL.match(calls[i + 1]), "the call after the write is not the kill")


def check_yield_then_kill(calls):
    """Yield, alive check, kill: nothing else in between."""
    idx = [i for i, c in enumerate(calls) if KILL_CALL.match(c)]
    check(len(idx) == 1, "expected one kill in the launcher's calls, got %d" % len(idx))
    i = idx[0]
    check(i > 1 and ALIVE_CHECK.match(calls[i - 1]), "the call before the kill is not the alive check")
    check(calls[i - 2].startswith("sched_yield()"), "the call before the alive check is not sched_yield")


def test_strace_default_path_yields_before_the_kill():
    if STRACE is None:
        check(os.environ.get("DS_REQUIRE_STRACE") != "1", "strace is required but missing")
        print("SKIP strace not available")
        return
    check_yield_then_kill(_strace_run([]))


def _expect_failure(fn, calls):
    try:
        fn(calls)
    except AssertionError:
        return
    raise AssertionError("the strace check accepted a bad trace")


def test_strace_checks_on_sample_traces():
    """The trace checks run everywhere, on sample strace -ff output of the launcher, so a Mac run exercises them."""
    wait = "wait4(101, 0x7ffd, WNOHANG, NULL) = 0"
    ordinary = [
        'execve("/x/build/dsl", ["dsl"], 0x7ffd) = 0',
        "nanosleep({tv_sec=0, tv_nsec=168000000}, NULL) = 0",
        "sched_yield() = 0",
        wait,
        "kill(101, SIGKILL <unfinished ...>",
        "--- SIGCHLD {si_signo=SIGCHLD} ---",
        "<... kill resumed>) = 0",
        "+++ exited with 0 +++",
    ]
    raw = [
        'execve("/x/build/dsl", ["dsl"], 0x7ffd) = 0',
        "nanosleep({tv_sec=0, tv_nsec=50000000}, NULL) = 0",
        "stat(\"/t/hairspring-region/n1.i0\", {st_mode=S_IFREG}) = 0",
        wait,
        'write(1, "DSC0 v1 role=sup pre-kill\\n", 26) = 26',
        "kill(101, SIGKILL) = 0",
    ]
    check_yield_then_kill(_launcher_calls(ordinary))
    check_write_then_kill(_launcher_calls(raw))
    # Each mutant of the order must be rejected.
    no_yield = [c for c in ordinary if not c.startswith("sched_yield")]
    _expect_failure(check_yield_then_kill, _launcher_calls(no_yield))
    no_alive = [c for c in ordinary if c != wait]
    _expect_failure(check_yield_then_kill, _launcher_calls(no_alive))
    extra = list(ordinary)
    extra.insert(extra.index(wait) + 1, "getpid() = 100")  # a call between the alive check and the kill
    _expect_failure(check_yield_then_kill, _launcher_calls(extra))
    swapped = list(raw)
    swapped[3], swapped[4] = swapped[4], swapped[3]  # write before the alive check
    _expect_failure(check_write_then_kill, _launcher_calls(swapped))
    between = list(raw)
    between.insert(5, "getpid() = 100")
    _expect_failure(check_write_then_kill, _launcher_calls(between))


def _launch_fields(r):
    return kv([l for l in r.stdout.splitlines() if " role=launch " in l][0])


def test_a_gap_in_the_sequence_numbers_makes_seq_ok_zero():
    r = run_n0("c3e", extra=["--server-arg=c1:--test-skip-seq"])
    f = _launch_fields(r)
    check(r.returncode == 3 and f["seq_ok"] == "0" and f["ok"] == "0", "a seq gap was not caught: %r" % f)
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2])


def test_a_child_left_alive_makes_live_after_one():
    r = run_n0("c3e", extra=["--test-leak-child"])
    f = _launch_fields(r)
    check(r.returncode == 3 and f["live_after"] == "1" and f["ok"] == "0", "a live child was not caught: %r" % f)
    time.sleep(2)  # the untracked child ends by itself
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2])


def test_dskv_drains_what_is_already_delivered_before_it_quits():
    port, peer_port = PORT_BASE + 28, PORT_BASE + 29
    peer = _listener(peer_port)
    server = subprocess.Popen(
        [exe("dskv"), "--mode=epoll", "--node=n1", "--port=%d" % port, "--peer-port=%d" % peer_port],
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        succ, _ = peer.accept()
        _read_lines(succ, 1)
        quitter = socket.create_connection(("127.0.0.1", port), timeout=10)  # the QUIT arrives in the same wake-up as the writes
        writer = socket.create_connection(("127.0.0.1", port), timeout=10)
        time.sleep(0.3)  # both accepted
        os.kill(server.pid, signal.SIGSTOP)
        quitter.sendall(b"QUIT\n")
        writer.sendall(b"REPL k5 9\n" * 100)  # more than one 512-byte read of the connection buffer
        time.sleep(0.2)
        os.kill(server.pid, signal.SIGCONT)
        out, _ = server.communicate(timeout=20)
    finally:
        if server.poll() is None:
            server.kill()
            server.wait()
        peer.close()
    f = kv(out.strip())
    check(f["repl_in"] == "100" and f["state"] == "%016x" % oracle.state_digest({5: 9}), "REPL sent before QUIT was lost: %r" % out)


def test_long_paths_are_refused_or_work_never_truncated():
    with Trial() as base:
        long_dir = os.path.join(base, "d" * 150)
        os.mkdir(long_dir)
        for name in ("dsl", "dskv", "dscli"):
            os.symlink(exe(name), os.path.join(long_dir, name))
        r = run([exe("dsl"), "--topo=c3e", "--bin-dir=" + long_dir, "--tmp=" + os.path.join(base, "t"), "--port-base=%d" % PORT_BASE])
        check(r.returncode == 0 and " ok=1" in r.stdout, "a 150-character bin-dir should work: rc %d %r" % (r.returncode, r.stdout[-200:]))
        too_long = os.path.join(base, "x" * 600)
        r = run([exe("dsl"), "--topo=c3e", "--bin-dir=" + too_long, "--tmp=" + os.path.join(base, "t2"), "--port-base=%d" % PORT_BASE])
        check(r.returncode == 2 and r.stdout == "" and "too long" in r.stderr, "a 600-character path should be refused: rc %d %r" % (r.returncode, r.stderr))
        r = run([exe("dsl"), "--topo=c3e", "--bin-dir=" + BIN, "--tmp=" + too_long, "--port-base=%d" % PORT_BASE])
        check(r.returncode == 2 and r.stdout == "", "a 600-character --tmp should be refused: rc %d" % r.returncode)
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2])


def test_bad_node_and_tag_names_are_refused_before_any_path_is_built():
    with Trial() as tmp:
        for d in ("ds-data", "hairspring-region"):
            os.mkdir(os.path.join(tmp, d))
        cases = [("n1", "n2.i0"), ("n1", "n1.i"), ("../x", "../x.i0"), ("n1", "n1.i0/../../y"), ("n/1", "n/1.i0")]
        for node, tag in cases:
            r = run([exe("dsnode"), "--node=" + node, "--tmp=" + tmp, "--recover-only"], env={"HAIRSPRING_PROC": tag})
            check(r.returncode == 2 and r.stdout == "", "node %r tag %r should be refused: rc %d" % (node, tag, r.returncode))
        check(os.listdir(os.path.join(tmp, "ds-data")) == [], "a refused name still created a file")
    r = run([exe("dskv"), "--mode=block", "--node=../x", "--port=%d" % (PORT_BASE + 30)])
    check(r.returncode == 2, "dskv should refuse a bad node name")


def test_a_group_kill_that_only_killed_the_pid_is_caught():
    """Regression mutant: dsl built so that a group-kill trial kills the pid alone must fail the group checks."""
    with Trial() as tmp:
        mutant = _build_c(tmp, os.path.join(ROOT, "c", "dsl.c"), ["-DDS_MUTANT_PID_KILL"], "dsl-mutant")
        args = [mutant, "--crash=pgid,1", "--helper", "--bin-dir=" + BIN, "--port=%d" % CRASH_PORT, "--seed=620001", "--pace-ms=10", "--tmp=" + os.path.join(tmp, "t")]
        rc, out = _trial_to_file(args, tmp)
    lines, killed, exited, sups, launch = _split(out)
    check(sups[0]["grp_gone"] == "0", "the helper survived a pid-only kill, so grp_gone must be 0: %r" % sups)
    helper = [e for e in exited if e["target"] == "n1.i0.h"]
    check(len(helper) == 1 and helper[0]["cause"] == "stop_kill", "the surviving helper must be stopped by the launcher: %r" % helper)
    check(rc == 3 and launch["ok"] == "0" and int(launch["unexpected"]) >= 1, "the launcher must report ok=0: rc %d %r" % (rc, launch))
    check_no_leftovers([CRASH_PORT])


def test_no_process_left_behind():
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2, CRASH_PORT])


TESTS = [
    test_operation_lists_match_golden_and_oracle,
    test_crash_plan_matches_golden_and_oracle,
    test_the_seed_comes_only_from_the_seed_argument,
    test_dskv_protocol_against_a_scripted_peer,
    test_dskv_retries_a_connect_made_before_the_peer_listens,
    test_dskv_block_mode_serves_one_connection_at_a_time,
    test_dscli_history_against_a_scripted_server,
    test_dscli_retries_connect_before_listen,
    test_dsnode_recovery_counts_records_and_torn_tails,
    test_dsnode_serves_and_leaves_an_intact_region,
    _topology_test("c3e", "epoll", True),
    _topology_test("c3t", "timeout", True),
    _topology_test("c3t2", "timeout", True),
    _topology_test("c3b", "block", False),
    test_topologies_produce_different_application_results_across_runs,
    test_early_exit_of_a_server_gives_ok0_and_rc3,
    test_a_gap_in_the_sequence_numbers_makes_seq_ok_zero,
    test_a_child_left_alive_makes_live_after_one,
    test_dskv_drains_what_is_already_delivered_before_it_quits,
    test_long_paths_are_refused_or_work_never_truncated,
    test_bad_node_and_tag_names_are_refused_before_any_path_is_built,
    test_a_group_kill_that_only_killed_the_pid_is_caught,
    test_a_client_with_failed_operations_exits_3_in_both_modes,
    test_pure_helpers_and_the_output_and_path_guards,
    test_a_victim_that_died_before_the_kill_is_not_reported_as_killed,
    test_an_unexpected_death_of_the_final_incarnation_is_counted,
    test_a_client_that_exits_3_makes_the_trial_not_ok,
    test_the_write_then_kill_trial_fails_when_the_node_never_started,
    test_an_alive_check_error_is_not_alive_and_never_signals_the_pid,
    test_an_unreaped_victim_gets_no_killed_line_and_stops_the_plan,
    test_a_server_that_ignores_quit_is_killed_and_counted,
    _crash_test("k1", 1),
    _crash_test("kp", 1, kind="pgid", flags=["--helper"]),
    _crash_test("kt", 1, flags=["--threads=2"]),
    _crash_test("k2", 2),
    test_crash_kills_land_in_the_workload,
    test_crash_process_group_kill_removes_the_helper,
    test_crash_write_then_kill_diagnostic_trial,
    test_strace_checks_on_sample_traces,
    test_strace_raw_write_is_immediately_followed_by_kill,
    test_strace_default_path_yields_before_the_kill,
    test_no_process_left_behind,
]

if __name__ == "__main__":
    raise SystemExit(run_tests(TESTS))
