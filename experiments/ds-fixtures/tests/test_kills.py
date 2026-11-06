"""Native kill stress: SIGKILL a serving node at pseudo-random points and check the report-region invariant.

For every kill: the region file must have an intact header, and with W = records in the log written by that
incarnation: done <= W <= started, started - done <= 1, acked <= done, done - acked <= 1, and the log must have no torn
entry (every record is one write(2)). The delays come from a fixed-seed generator, so a failure repeats.

The default count is small enough for a laptop (DS_KILLS, 100); the experiment's acceptance run is DS_KILLS=1000 for
dsnode and DS_GO_KILLS=200 for dsgonode on Linux. This is evidence about the fixtures only, not about Hermit.
"""
import os
import random
import signal
import socket
import subprocess
import threading
import time

import oracle
from dstest import CRASH_PORT, GO_KILLS, KILLS, Trial, check, check_no_leftovers, exe, read_bytes, run_tests

STORM_BYTES = b"".join(b"PUT k%d %d\n" % (i % 8, 1000 + i % 900) for i in range(20000))


def _connect(port):
    for _ in range(200):
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=5)
        except OSError:
            time.sleep(0.01)
    raise AssertionError("the node never listened")


def _kill_once(program, rng, port):
    with Trial() as tmp:
        for d in ("ds-data", "hairspring-region"):
            os.mkdir(os.path.join(tmp, d))
        node = subprocess.Popen(
            [exe(program), "--node=n1", "--port=%d" % port, "--tmp=" + tmp],
            stdout=subprocess.DEVNULL,
            env=dict(os.environ, HAIRSPRING_PROC="n1.i0"),
        )
        try:
            conn = _connect(port)
            delay = rng.uniform(0.0, 0.03)
            timer = threading.Timer(delay, lambda: os.kill(node.pid, signal.SIGKILL))
            timer.start()
            try:
                conn.sendall(STORM_BYTES)
            except OSError:
                pass
            node.wait(timeout=30)
            timer.cancel()
            conn.close()
        finally:
            if node.poll() is None:
                node.kill()
                node.wait()
        check(node.returncode == -signal.SIGKILL, "the node was not killed by SIGKILL: rc %r" % node.returncode)
        records, torn = oracle.parse_log(read_bytes(os.path.join(tmp, "ds-data", "n1.log")))
        check(torn == 0, "the log has %d torn entries" % torn)
        region = oracle.parse_region(read_bytes(os.path.join(tmp, "hairspring-region", "n1.i0")))
        bad = oracle.region_invariant(region, 0, records, killed=True)
        check(bad == "", "region invariant broken (%s): %r with %d records" % (bad, region, len(records)))
        return len(records)


def _stress(program, count, seed):
    rng = random.Random(seed)
    written = [_kill_once(program, rng, CRASH_PORT) for _ in range(count)]
    check(any(w > 0 for w in written), "no kill ever came after a write: the stress did not exercise the region")
    check_no_leftovers([CRASH_PORT])
    print("INFO %s: %d kills, records written before the kill: min %d max %d" % (program, count, min(written), max(written)))


def test_dsnode_region_invariant_after_native_kills():
    _stress("dsnode", KILLS, 20261105)


def test_dsgonode_region_invariant_after_native_kills():
    _stress("dsgonode", GO_KILLS, 20261106)


if __name__ == "__main__":
    raise SystemExit(run_tests([test_dsnode_region_invariant_after_native_kills, test_dsgonode_region_invariant_after_native_kills]))
