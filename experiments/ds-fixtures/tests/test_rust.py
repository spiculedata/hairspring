"""Native self-test of the Rust fixture (dsrs) inside the C launcher. Not run under Hermit."""
from dstest import PORT_BASE, RUNS, check_n0_ok, check_no_leftovers, run_n0, run_tests


def test_topology_r3_all_runs_ok():
    for _ in range(RUNS):
        check_n0_ok("r3", run_n0("r3"), conserve=False, mode="rust")
    check_no_leftovers([PORT_BASE, PORT_BASE + 1, PORT_BASE + 2])


if __name__ == "__main__":
    raise SystemExit(run_tests([test_topology_r3_all_runs_ok]))
