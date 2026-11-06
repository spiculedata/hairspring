"""Native self-test of the Go fixtures (dsgo, dsgonode) inside the C launcher. Not run under Hermit."""
from dstest import (
    CRASH_PORT,
    PORT_BASE,
    RUNS,
    Trial,
    check_c0_trial,
    check_n0_ok,
    check_no_leftovers,
    run_c0,
    run_n0,
    run_tests,
)


def test_topology_g3_all_runs_ok():
    for _ in range(RUNS):
        check_n0_ok("g3", run_n0("g3"), conserve=False, mode="go")
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2])


def test_crash_kg_all_runs_ok():
    for i in range(RUNS):
        seed = 620001 + i
        with Trial() as tmp:
            r = run_c0(seed, 1, node_bin="dsgonode", tmp=tmp)
            check_c0_trial(r, tmp, seed, 1)
    check_no_leftovers([CRASH_PORT])


def test_go_node_with_two_kills():
    with Trial() as tmp:
        r = run_c0(620001, 2, node_bin="dsgonode", tmp=tmp)
        check_c0_trial(r, tmp, 620001, 2)
    check_no_leftovers([CRASH_PORT])


if __name__ == "__main__":
    raise SystemExit(run_tests([test_topology_g3_all_runs_ok, test_crash_kg_all_runs_ok, test_go_node_with_two_kills]))
