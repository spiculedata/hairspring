"""Native self-test of the C fixtures (dsl, dskv, dscli, dsnode). Not run under Hermit.

Covers: operation-list and crash-plan golden files against the independent oracle, protocol behaviour of dskv and
dsnode against a scripted peer, the dscli history file against a scripted server, recovery of the node log,
every N0 topology and every C0 topology RUNS times, the launcher's early-exit and straggler handling, and (Linux with
strace) the syscall adjacency the kill supervisor promises.
"""
import os
import re
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time

import oracle
from dstest import (
    BIN,
    CRASH_PORT,
    HERE,
    PORT_BASE,
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


def test_plan_takes_its_seed_from_the_environment():
    r = run([exe("dsl"), "--print-plan", "--kills=1"], env={"HAIRSPRING_SEED": "620001"})
    check(r.stdout.splitlines() == oracle.plan_lines(620001, 1), "HAIRSPRING_SEED was not used")
    env = {k: v for k, v in os.environ.items() if k != "HAIRSPRING_SEED"}
    r = subprocess.run([exe("dsl"), "--print-plan", "--kills=1"], capture_output=True, text=True, env=env)
    check(r.returncode == 2 and r.stdout == "", "a missing seed must be a usage error with no output")


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
    # Both clients write the same keys, so the final state can differ across host schedules; this is a native check
    # that the workload exposes interleavings, not a claim about any other environment.
    states = set()
    for r in _repeat("c3e", max(RUNS, 20)):
        states.add(tuple(kv(l)["state"] for l in sorted(r.stdout.splitlines()) if " role=srv " in l))
    check(len(states) >= 1, "no state seen")
    print("INFO distinct native final states in c3e: %d" % len(states))


def test_early_exit_of_a_server_gives_ok0_and_rc3():
    r = run_n0("c3e", extra=["--server-arg=n2:--test-exit-early"])
    check(r.returncode == 3, "rc should be 3, got %d" % r.returncode)
    launch = [l for l in r.stdout.splitlines() if " role=launch " in l][0]
    f = kv(launch)
    check(f["ok"] == "0" and f["live_after"] == "0" and "n2:1" in f["exits"], "bad launcher line %r" % launch)
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
        check(lines[0] == "DSC0 v1 role=node tag=n1.i0 recovered=0 torn=0", "bad first line %r" % lines)
        check(lines[1] == "DSC0 v1 role=sup pre-kill" and len(lines[1]) + 1 == 26, "bad pre-kill line %r" % lines[1])
        check(lines[2].startswith("HAIRSPRING_EVENT ") and ' role=sup kill=1 victim=n1.i0 wifsig=1 termsig=9 ' in lines[3], "bad kill lines %r" % lines)
        check(re.match(r"^DSC0 v1 role=launch kills=1 restarts=0 stragglers=0 live_after=0 seq_ok=1 hist=[0-9a-f]{16} ok=1$", lines[4]), "bad launcher line %r" % lines[4])
        check(len(lines) == 5, "unexpected extra lines %r" % lines)
    check_no_leftovers([CRASH_PORT])


STRACE = shutil.which("strace")


def _strace_run(flags):
    with Trial() as tmp:
        out = os.path.join(tmp, "trace.txt")
        args = [STRACE, "-f", "-o", out, exe("dsl"), "--crash=pid,1", "--bin-dir=" + BIN, "--port=%d" % CRASH_PORT, "--seed=620001", "--tmp=" + os.path.join(tmp, "t")] + flags
        os.mkdir(os.path.join(tmp, "t"))
        r = run(args)
        check(r.returncode == 0, "strace run rc %d: %r" % (r.returncode, r.stderr[-300:]))
        with open(out, errors="replace") as f:
            return f.read().splitlines()


def _launcher_calls(trace):
    """The syscall lines of the launcher (the first pid in the trace), in order."""
    pid = trace[0].split()[0]
    calls = [l.split(None, 1)[1] for l in trace if l.split()[0] == pid and len(l.split()) > 1]
    return [c for c in calls if not c.startswith(("---", "+++"))]


def test_strace_raw_write_is_immediately_followed_by_kill():
    if STRACE is None:
        check(os.environ.get("DS_REQUIRE_STRACE") != "1", "strace is required but missing")
        print("SKIP strace not available")
        return
    check_write_then_kill(_launcher_calls(_strace_run(["--pause", "--raw-write-kill"])))


def check_write_then_kill(calls):
    idx = [i for i, c in enumerate(calls) if c.startswith('write(1, "DSC0 v1 role=sup pre-kill\\n", 26)')]
    check(len(idx) == 1, "expected exactly one pre-kill write in the launcher's calls")
    check(
        idx[0] + 1 < len(calls) and re.match(r"kill\(\d+, SIGKILL\)", calls[idx[0] + 1]),
        "the call after the write is not the kill",
    )


def check_yield_then_kill(calls):
    idx = [i for i, c in enumerate(calls) if re.match(r"kill\(\d+, SIGKILL\)", c)]
    check(len(idx) == 1, "expected one kill in the launcher's calls, got %d" % len(idx))
    check(idx[0] > 0 and calls[idx[0] - 1].startswith("sched_yield()"), "the call before the kill is not sched_yield")


def test_strace_default_path_yields_before_the_kill():
    if STRACE is None:
        check(os.environ.get("DS_REQUIRE_STRACE") != "1", "strace is required but missing")
        print("SKIP strace not available")
        return
    check_yield_then_kill(_launcher_calls(_strace_run([])))


def _expect_failure(fn, calls):
    try:
        fn(calls)
    except AssertionError:
        return
    raise AssertionError("the strace check accepted a bad trace")


def test_strace_checks_on_sample_traces():
    """The trace checks run everywhere, on sample strace -f output, so a Mac run still exercises their logic."""
    trace = [
        '100 execve("dsl", ["dsl"], 0x7ffd) = 0',
        '101 execve("dsnode", ["dsnode"], 0x7ffd) = 0',
        "100 nanosleep({tv_sec=0, tv_nsec=168000000}, NULL) = 0",
        "100 sched_yield() = 0",
        '100 write(1, "DSC0 v1 role=sup pre-kill\\n", 26) = 26',
        "101 pause() = ? ERESTARTNOHAND (To be restarted if no handler)",
        "100 kill(101, SIGKILL) = 0",
        "100 --- SIGCHLD {si_signo=SIGCHLD} ---",
        "101 +++ killed by SIGKILL +++",
    ]
    calls = _launcher_calls(trace)
    check_write_then_kill(calls)
    check_yield_then_kill([c for c in calls if not c.startswith("write")])
    _expect_failure(check_yield_then_kill, calls)  # a write sits between the yield and the kill
    bad = list(calls)
    bad.insert(calls.index("kill(101, SIGKILL) = 0"), "getpid() = 100")
    _expect_failure(check_write_then_kill, bad)


def test_no_process_left_behind():
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2, CRASH_PORT])


TESTS = [
    test_operation_lists_match_golden_and_oracle,
    test_crash_plan_matches_golden_and_oracle,
    test_plan_takes_its_seed_from_the_environment,
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
