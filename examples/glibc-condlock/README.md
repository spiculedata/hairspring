# condlock

A small C workload for a glibc condition-variable lost-wakeup of the kind described in glibc bug BZ 25847, together with
the self-tests of its stuck-state detector.

N worker threads contend for a lock built from a mutex and a condition variable (untimed `pthread_cond_wait`, an
unconditional `pthread_cond_signal` on release), meet at a barrier every round, and a watchdog thread decides whether a
signal was lost. It reports through the repo's C SDK ([`sdks/c`](../../sdks/c), pinned in `PINS`).

## What it checks

A releaser holding the lock's own mutex records which workers were blocked in `pthread_cond_wait` when it sent a signal.
The watchdog samples progress on a fixed tick. After a quorum of ticks without progress it takes the lock's mutex (by
trylock and short sleeps, never an untimed lock) and applies one rule: the lock is free, the recorded set is not empty,
and every worker in that set is still blocked in the same wait. When the rule holds, the property
`glibc.cond.signal_wakes_one_of_blocked` is false, the run prints a state dump and ends.

If the process is quiescent but the rule does not hold, the run is a stall, not a property failure. Stall subtypes:

| Subtype | Meaning |
| --- | --- |
| Q | lock free, at least one worker still blocked on the lock condition variable (rule did not hold) |
| L | lock held (leaked or held): nobody can make progress |
| B | lock free, no lock waiter, barrier incomplete |
| M | the lock's mutex could not be taken for a whole tick |

```text
condlock [-n N] [-r R] [-tick-ms T] [-q Q]     defaults 8, 500, 100, 10
exit status: 0 completed, 3 property false, 4 stall, 5 harness invariant, 64 usage (never 1)
```

`scripts/classify.sh` turns one run's exit status and outputs into a class.

## What is needed to run it

- A glibc built from the unmodified `release/2.40` source line. Build note, exactly: glibc built with
  `-U_FORTIFY_SOURCE` added to CFLAGS (upstream's default no-fortify build; same effect as upstream fix 127cefd,
  BZ 31928), Ubuntu gcc 13.3.0 defaults otherwise.
- The workload linked statically against that glibc. The glibc-specific state dump (`src/cl_condraw.c`) is compiled only
  for a glibc build and reads the public `__data` view of `pthread_cond_t`.
- Native runs on the machine that built it. Native runs on macOS use the macOS pthread library, not glibc, and are
  informational only.
- Optionally, running the same binary under Hermit on Linux x86-64.

## Arms

| Arm | What it is |
| --- | --- |
| Unmodified source line | `release/2.40` before the BZ 25847 backport. |
| Fixed branch (control) | The same line with the BZ 25847 fix, to check whether the property stays true where the fix is present. |
| Positive control (assisted) | The unmodified line plus one patch hunk that adds a short delay in the signal path. This arm is assisted: the delay is placed by the person setting it up and is not part of upstream glibc. |

## Labels

- The bug is known in advance.
- The workload is targeted at that bug.
- Its structure is taken from a public reproducer.
- The workload is instrumented with the protocol SDK (`sdks/c`).

## Building and self-tests

```sh
make condlock tprobe selftest-bins   # the counted build, the T-probe, the planted and variant builds
make test                            # macOS only: unit tests, workload behaviour, classifier, T-probe, results guard
sh scripts/run-selftests.sh all      # macOS only: D-pos / D-neg / D-stall into a NEW results/m1-mac-<stamp> directory
sh scripts/pack-results.sh results/m1-mac-<stamp>   # macOS only (BSD tar flags)
make unit                            # unit tests (rule, config, report, state dump); works on macOS and Linux
```

`make test`, `scripts/run-selftests.sh`, `scripts/pack-results.sh` and `scripts/record-build.sh` are macOS-only: they
use `sw_vers`, `sysctl`, `shasum` and BSD `tar` flags, and `tests/test_workload.sh` expects the non-glibc library
(`libc=none hdr=none`). `make unit` (102 checks) runs on Linux too, as does the T-probe check (`tests/test_tprobe.sh`, after `make tprobe`);
both were run on Ubuntu 24.04. The classifier checks (`tests/test_classify.sh`) also run there, but their `real_*`
cases need the built workload binaries. CI runs `make unit` only.

The planted builds (`-DCONDLOCK_DROP_SIGNAL=k`, `-DCONDLOCK_PLANT_LEAK=k`, `-DCONDLOCK_PLANT_BARRIER=k`,
`-DCONDLOCK_PLANT_HOLDG=k`) are separate binaries used only to test the detector; the counted binary contains no planted
code. `run-selftests.sh` refuses to write into an existing results directory. Its results, written under `results/` and
not committed here, are detector self-tests on a non-glibc library and say nothing about glibc.

## Results

The experiment that used this workload is described in
[Pinning glibc to test a lost wakeup: what we could and could not measure](../../docs/showcase-glibc-lost-wakeup.md).
In short, on one machine (Ubuntu 24.04.5, kernel 6.17.0-42-generic, Intel Xeon E5-1410 v2, gcc 13.3.0), natively,
the property site was false at least once in 1,025 of 2,029 runs on the unmodified `release/2.40/master` state
`d8a1a1ae` (unassisted) and in 0 of 2,029 runs on the fixed state `8d3dd23e` (fixed state compared at arm level
only). Labels: native reference; bug known in advance; targeted workload; structure from a public reproducer;
environment pinned; instrumented (built with the protocol SDK). Two positive controls (the unmodified library plus
one hunk that widens the race window; assisted, window_widening, library_patched) did not make the property false in
any of the runs made under the pinned Hermit (P1: 90 runs including 30 informational; P2: 60), so the counted runs
under Hermit were not started, and nothing is claimed about whether Hermit reaches the bug's state.
glibc built with -U_FORTIFY_SOURCE added to CFLAGS (upstream's default no-fortify build; same effect as upstream fix
127cefd, BZ 31928), Ubuntu gcc 13.3.0 defaults otherwise.

These results were produced from a pre-rebase local commit. That commit's `src/` differs from this example's in one
comment only. Outside `src/`, `PINS` names an earlier C SDK commit (an ancestor of the one pinned here, with unchanged
SDK library source), and `scripts/pack-results.sh` (one tar flag) and `scripts/record-build.sh` (one comment) differ.
This example rebuilds the workload, with the Linux binaries from the scripts in [`experiment/`](experiment). Builds are
not byte-reproducible across rebuilds or build paths, so compare stripped objects, not file hashes. `BUILD-RECORD` and
`BUILD-RECORD-fix1` name the commit the macOS binaries were built from before squashing (commits that are not in this
repository's history).

## Known limitations of the example

- The watchdog ends the process with `hs_exit` while worker threads may still be running, so the exit path can race
  with them (`src/cl_watchdog.c`, `src/main.c`).
- The state dump in `src/cl_condraw.c` reads the glibc condition-variable internals with plain loads, not atomic ones,
  so a dump taken while the variable is changing can be inconsistent; it is a description, not part of the property.
- The worker count is capped at 16 (`-n`, `src/cl_config.c`).
- `make test`, `scripts/run-selftests.sh` and `scripts/record-build.sh` are macOS-only (see above), and the Makefile
  does not rebuild the C SDK library when the SDK sources change; run `make clean` first.
