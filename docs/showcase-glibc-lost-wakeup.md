# Pinning glibc to test a lost wakeup: what we could and could not measure

This page describes an experiment that set out to run a known glibc condition-variable bug (glibc bug 25847, a lost
wakeup fixed upstream in 2025) under Hermit, the deterministic scheduler behind Hairspring's `process` backend, and that
stopped, by its own pre-registered rule, before the counted runs. It is the companion of the
[Go walkthrough](showcase-go-sql-cancel-race.md), which reached its counted runs; this one did not, and says why.

What you can take from it:

- how to build and pin a glibc source state that no distribution ships, and prove that a test program is linked against
  that glibc and nothing else;
- a test workload and a stuck-state property designed not to fire falsely under Hermit's time model, with planted faults
  that check the detector fires when it should;
- a native observation on one machine: on the unmodified source state the property site was false at least once in
  1,025 of 2,029 runs, and on the fixed source state in none of 2,029 runs (labels and build note in 5.2);
- per-seed determinism checks of the workload under the pinned Hermit;
- two positive controls (the unmodified library plus one hunk that widens the race window) that did not make the
  property false in any of the runs made under the pinned Hermit (P1: 90 runs including 30 informational; P2: 60),
  which by the pre-registered rule ended the experiment.

What it does not have: any statement about whether Hermit reaches the bug's state on the unmodified library, at any
frequency. The counted runs that would have answered that were never started.

The scripts, patches and the Hermit build recipe are in
[`examples/glibc-condlock/experiment`](../examples/glibc-condlock/experiment); the workload is in
[`examples/glibc-condlock`](../examples/glibc-condlock).

## 1. The bug

glibc's condition variable splits waiters into two groups, G1 and G2, and moves group slots forward as signals are
delivered. In the source state used here (`release/2.40/master` at `d8a1a1ae`), a waiter that wakes up takes a signal
with a compare-and-swap on its group's signal counter, and only then loads `__g1_start` to check whether the signal it
took belonged to a newer group that now occupies the same slot. If it decides it "stole" a signal, it puts one back.

The bug report (BZ 25847, opened 2020-04-18) describes the bad interleaving: the waiter is descheduled between the
compare-and-swap and the `__g1_start` load, the group slots rotate twice while it is off the CPU, and on resuming it
wrongly concludes that it stole from the current G1. It returns a signal to G1 without adjusting G1's waiter count. Later
`pthread_cond_signal` calls then deliver a signal to a G1 that has no waiter left, while real waiters in G2 stay asleep.
The program sees a thread that sleeps forever after a signal was sent.

The upstream fix (nine commits, authored 2024-12-04, committed 2025-01-17) removes signal stealing altogether and makes
the per-group signal count relative to `__g1_start`. It was first released in 2.41 and backported to the 2.38 to 2.40 release branches. On
`release/2.40/master` the backport plus one initializer-compatibility commit is the range `d8a1a1ae..8d3dd23e`: ten
commits whose `git patch-id --stable` values equal those of the corresponding master commits, one by one.

The window in question is a few instructions with no system call in it. [I] On a real machine it needs a preemption at
exactly that point plus two group rotations before the thread runs again; native observations reported in the bug
thread used many instances on dedicated hardware and tuned scheduler settings.

Estimates and inferences on this page are marked [I]. Everything else is either background from the public bug report
and commits, or an observation from a pre-registered run.

## 2. The test workload and its property

### 2.1 What the workload does

The workload (`condlock`) is new code, written for this experiment. Its structure follows a public reproducer attached
to the bug report; no code was copied from it.

- Eight worker threads share a lock built from a mutex and a condition variable: `acquire` waits with an untimed
  `pthread_cond_wait` while the lock is held; `release` clears the flag and calls `pthread_cond_signal`
  unconditionally, as the public reproducer does.
- Each round, every worker acquires the lock, runs a fixed loop of 1,000 increments of a volatile counter, releases the
  lock, and waits at a barrier built from a second mutex and condition variable (so a lock signal can never be consumed
  by a barrier waiter).
- One watchdog thread sleeps on a fixed tick and is the only thread that reads time. It never blocks without a timeout:
  it takes the lock's mutex by `trylock` plus short sleeps.
- Flags: `-n` workers (8), `-r` rounds, `-tick-ms` tick length, `-q` quorum of ticks (10).
- The workload links statically against the glibc under test and reports through Hairspring's C SDK
  ([`sdks/c`](../sdks/c), [docs/c-sdk.md](c-sdk.md)). Builds with the SDK are labelled `T2_instrumented`: the SDK's
  startup and exit writes change the schedule compared with an unreported build.
- Exit statuses: 0 completed, 3 property false (the watchdog ends the process, because the workers are blocked for
  good), 4 stall, 5 harness invariant, 64 usage. It never exits with status 1.

No Hairspring engine feature (targets, sweeps, bundles, replay) was used for any number on this page: every run was
driven by the scripts in `examples/glibc-condlock/experiment`, and hits were counted from each run's own SDK report.

### 2.2 The property and why it is designed not to fire falsely

A releaser, holding the lock's mutex, records the set B of workers that are blocked in `pthread_cond_wait` at the moment
it signals, and how many times each has returned. The watchdog counts ticks without progress (lock acquisitions plus
barrier arrivals). After Q consecutive ticks without progress it takes the mutex and evaluates:

> the lock is free, B is not empty, and every worker in B is still blocked in the same wait it was in when the signal
> was sent.

When that holds, the site `glibc.cond.signal_wakes_one_of_blocked` is false. POSIX requires a signal to unblock at least
one thread blocked on the condition variable at that moment, so a woken member of B must eventually take the free mutex
and return, which would make the rule false. For the rule to fire falsely, a runnable thread would have to stay
unscheduled for Q ticks while nothing else makes progress.

[I] Under Hermit's time model, virtual time advances only through execution, scheduler turns and system calls, or by
jumping to the next timed deadline when nothing is runnable; a starved-but-runnable thread for a whole virtual second
(Q = 10 ticks of 100 ms) would need the other threads to execute for that long without making progress. This is the
design argument. It is backed by a measured margin: in the healthy runs of the determinism check, the longest run of
stagnant ticks must be at most 3 of the 10 needed.

Natively the rule is a heuristic: an operating-system scheduler can starve a runnable thread. False fires of the native
heuristic were therefore counted on the fixed library (section 5.2).

What the property can miss: a waiter already woken by an earlier signal that has not yet re-taken the mutex looks
blocked and hides a stuck state. That case, and other quiescent states, end as a **stall** (exit 4) with a subtype: Q
(lock free, a worker still blocked, rule not satisfied), L (lock held), B (barrier incomplete), M (the mutex could not
be taken for a whole tick). A stall is never counted as a hit.

On a hit or a stall the watchdog prints one line with its counters and the raw fields of the condition variable from the
public header, so a person can compare the end state with the state dumps in the bug report. This line is a description,
not part of the property and not evidence of cause.

### 2.3 Detector self-tests (planted builds)

Separate binaries, never counted as results, check the detector:

- **drop** (`-DCONDLOCK_DROP_SIGNAL=k`): from the k-th release onwards the lock condition variable is never signalled.
  The property must fire in every run where the plant was applied.
- **leak**, **barrier**, **holdg**: a leaked lock, a skipped barrier broadcast, a held mutex. Each must end as a stall
  of subtype L, B or M **with the property true**, so that a protocol-level deadlock is never reported as a lost wakeup.

These show the detector fires on an injected end state. They do not show that it would fire on the real bug.

### 2.4 The pinned Hermit

Hermit runs a Linux x86-64 program one thread at a time under a seeded scheduler with virtual time. In chaos mode it
ends timeslices at random points, measured in retired conditional branches, and gives threads random priorities. The
build used here is pinned by its sha256 (`666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a`,
`hermit 0.4.1 (2026-10-15)`): commit `25781c5` of the maintained fork with two replay commits cherry-picked on top; the
exact recipe is in the experiment folder. Every number on this page that was measured under Hermit belongs to that
build; [I] a different build may give different schedules for the same seed.

## 3. The library builds

Four glibc trees were built (two upstream source states and two patched variants of U), each into a private sysroot,
and the workload was linked statically against each:

| Arm | Source | Labels |
|---|---|---|
| U | `release/2.40/master` at `d8a1a1aef7a58b991505b9a1349a40736dec3abf` (before the backport) | `unassisted` in the native cell |
| C | `release/2.40/master` at `8d3dd23e3de8b4c6e4b94f8bbfab971c3b8a55be` (U plus the backported fix and the initializer-compatibility commit) | `fixed_branch_control`, at arm level only |
| P1 | U plus one hunk, a 10 ms clock_nanosleep between the CAS loop and the `__g1_start` load; positive control; assisted, window_widening, library_patched | |
| P2 | U plus one hunk (blob `e58d3b83`), a 40,000-iteration PAUSE busy-wait between the signal-grab CAS and the `__g1_start` load; positive control; assisted, window_widening, library_patched | |

Build note, exactly as it applies to every build on this page: glibc built with -U_FORTIFY_SOURCE added to CFLAGS (upstream's default no-fortify build; same effect as upstream fix 127cefd, BZ 31928), Ubuntu gcc 13.3.0 defaults otherwise.

The recipe: a private GNU bison 3.8.2 and m4 1.4.19 built from GPG-verified tarballs as an unprivileged user (the host
was not changed); glibc configured with `--prefix=/usr --libdir=/usr/lib --with-headers=<copied kernel headers>
--disable-werror --disable-profile --disable-nscd --without-selinux` and `CFLAGS="-O2 -g -U_FORTIFY_SOURCE
-ffile-prefix-map=<build dir>=/g0"`, built with `make -j3`, installed with `make install install_root=<sysroot>`. The
workload and the SDK were compiled with `gcc --sysroot=<sysroot> -B<sysroot>/usr/lib/ -static -L<sysroot>/usr/lib` and a
linker trace (`-Wl,-t,-t`).

Isolation was checked, not assumed: the link trace must name members of the arm's own `libc.a` and no system glibc
component, the binary must have no interpreter and no dynamic section, and at run time the workload prints
`gnu_get_libc_version()` and the header version, which must read `libc=2.40 hdr=2.40`.

Nothing on this page is a statement about any distribution's own glibc package.

Apart from the container pre-checks, every build and run on this page used one machine: Ubuntu 24.04.5, kernel
6.17.0-42-generic, x86-64 (Intel Xeon E5-1410 v2), gcc 13.3.0. The workload binaries were built from
a local commit made before this example was rebased onto the current repository history. That commit's `src/` is
identical to this example's apart from the C SDK pin (the SDK commit it used is an ancestor of the one pinned here, and
the SDK library source did not change between them) and one comment. This example rebuilds the workload: the macOS
build with `make`, and the Linux binaries with the scripts in
[`experiment/`](../examples/glibc-condlock/experiment). Binaries built from this example were not part of these runs,
and builds are not byte-reproducible across rebuilds or build paths (see 5.1 and section 7); compare stripped objects,
not file hashes.

## 4. How the experiment was designed and pre-registered

Each step was a separate specification with its scripts embedded and hashed. An independent review step signed each
experiment before it ran. Each specification fixed, before any of its data existed: the commands, the seeds, the
evaluator code, the outcome table (the first rule that holds gives the outcome), the stop rules, and the sentences that
each outcome allows. The evaluator, not a person, printed the outcome; results were copied, not re-judged. Hits were
counted per site from each run's own report, never from a pass/fail verdict of any tool.

The planned sequence of steps was:

1. build and isolation checks (container pre-checks, then the test machine);
2. a native reference on the test machine (no Hermit);
3. the planted detector builds;
4. the positive control under Hermit (a feasibility probe, then a gate: at least 3 hits in 200 seeds);
5. determinism, timing and detector gates under Hermit;
6. the counted runs: 2,000 seeds each of U and C, with fixed thresholds for a hit result (at least 8 of 2,000 on U with
   0 on C) and for a bounded null (0 of 2,000 on U);
7. confirmation of hits by direct reruns and replay.

Two rules matter for how this account ends:

- **No counted run without a positive control.** Both the hit result and the bounded null required the positive-control
  gate to have passed: a null on U means nothing unless the same setup demonstrably reports the state when the window
  is widened by hand.
- **No third positive control.** Before the first positive control's probe ran, the rule was recorded that zero hits in
  its 60 probe runs would require a new design under its own specification. That happened, and a second one was
  designed. The rule recorded before the second probe ran: at least one hit in its 60 probe runs and it is used; zero of
  60 and there is no third design, and the experiment stops.

What was not pre-registered from the start, stated plainly:

- The second positive control was designed **after** the first had returned zero. Its design used that zero and a
  reading of Hermit's source code; it did not use any run-level number to set its parameters.
- Its loop count was corrected once, from 10,000 to 40,000 iterations, **before any run of it under Hermit**. The
  reason was a build fact: the compiler unrolled the 10,000-iteration loop by two, so each pass had 5,000 conditional
  branches, half the window the design had specified. The pre-registered quantity was "conditional branches per pass at
  least one maximum timeslice (10,000)"; 40,000 iterations give 20,000. A few native smoke runs of the superseded
  10,000-iteration build existed when the count was changed (all completed normally); they were not used for the change.
  [I] With 5,000 branches per pass, a timeslice end inside a pass would have had a probability of roughly 0.39 to 0.62
  instead of the near-certainty the design assumed; this was recognised from the build fact, and the corrected count
  restored the designed quantity.
- The number of native runs in the native reference was set by a time window, not fixed in advance and not chosen from
  results.

## 5. Each step and its result

All results below are for one machine, one job per configuration, except where several launches are listed, on the
dates given (UTC).

### 5.1 Builds and isolation

Build note for every build in this section: glibc built with -U_FORTIFY_SOURCE added to CFLAGS (upstream's default no-fortify build; same effect as upstream fix 127cefd, BZ 31928), Ubuntu gcc 13.3.0 defaults otherwise.

The build recipe took several attempts on 2026-10-16 before it passed: the first stopped at the build-tool check (bison
missing, after which the private tool build was added); the next stopped at a compile error in `misc/syslog.c`; a third
built both arms but its link check could not read library members from the linker trace, after which the trace flag
became `-Wl,-t,-t`.

On 2026-10-17 (00:38) U and C built, a two-thread signal/wait smoke program linked statically against each completed
50 of 50 plain runs and 6 of 6 runs under the pinned Hermit, the link trace and ELF checks showed no system glibc
component, and the two `libc.a` files differed (after stripping debug information) in five members:
`pthread_cond_broadcast.o`, `pthread_cond_destroy.o`, `pthread_cond_init.o`, `pthread_cond_signal.o`,
`pthread_cond_wait.o`.

The first positive control was built in a linux/amd64 Ubuntu 24.04 container first: after stripping debug information,
U's and P1's `libc.a` differ in exactly `pthread_cond_wait.o` (2,204 members compared).

On the test machine (2026-10-17, 14:43 to 15:11) U, C and P1 were built in one job; each linked the workload statically,
with 534 member lines from the arm's own `libc.a`, 8 from the SDK archive and no trace violation; U against P1 differ in
exactly `pthread_cond_wait.o`; a 60-second virtual sleep under the pinned Hermit completed in 74 to 164 ms of wall time
on each arm (Hermit's fast-forward of an all-blocked process, which the property's design relies on). The
same job's plain-run cell recorded hit runs on U; by its pre-registered rule this stopped the native step until the
dedicated native experiment below was specified and signed.

The planted and violation-only builds (2026-10-17, 21:15): twenty planted builds (`drop`, `leak`, `barrier`, `holdg`,
each with k in 1, 2, 5, 20, 50) linked against arm C, and two violation-only builds (no SDK) against U and P1; rebuilt
once at the same path, 22 of 22 files were byte-identical, and their sha256 values were pinned. (The same job also
rebuilt the U, C and P1 workload binaries at a different path: each differed from the earlier build by 125 bytes and was
equal to it after stripping debug information and the build ID. Byte identity held only within one job at one path.)
Under the pinned Hermit the guest exit statuses of `condlock-drop-k1` (3) and `condlock-leak-k1` (4) came back
unchanged in 3 of 3 runs each.

### 5.2 Native reference (no Hermit)

Labels: native reference, not a claim about Hermit; bug_known_in_advance; targeted_workload;
structure_from_public_reproducer; environment_pinned; arm U unassisted; arms U, C and P1 T2_instrumented; arm C
fixed_branch_control at arm level only. Configuration: the binaries built in 5.1, run as an unprivileged user on three
cores at nice 10, one run at a time, 2026-10-17 19:01 to 20:21. Build note: glibc built with -U_FORTIFY_SOURCE added to CFLAGS (upstream's default no-fortify build; same effect as upstream fix 127cefd, BZ 31928), Ubuntu gcc 13.3.0 defaults otherwise.

Arms U and C ran in alternating rounds, one run of each per round, with `-n 8 -r 5000 -tick-ms 50 -q 10`:

- **Arm U:** 1,025 of 2,029 runs had at least one false evaluation of the property site (Wilson 95% interval
  [48.34%, 52.69%]).
- As a description only: all 1,025 state dumps of those runs have the shape that was pre-registered as "an unconsumed
  signal for an empty group" (one group holds a pending signal and has no waiter, every waiter sits in the other group,
  the lock is free). This is a printed state, not a cause.
- **Arm C:** 0 of 2,029 runs (exact one-sided 95% upper bound 0.1475%). The watchdog evaluated on average 16.98 ticks per
  run, so the zero is a non-vacuous measurement of false fires of the native heuristic on the fixed library.
- No run in either arm timed out, crashed, failed a harness check or ended in a stall.

The first positive control ran in its own cell with `-n 8 -r 60 -tick-ms 50 -q 10` (U plus one hunk, a 10 ms clock_nanosleep between the CAS loop and the `__g1_start` load; positive control; assisted, window_widening, library_patched): the property site was false in 0 of 200 native runs (exact one-sided 95% upper bound 1.4867%). It ran at 60 rounds, not 5,000, because every wait in that build sleeps 10 ms; it is never compared with U or C.

### 5.3 The first positive control under Hermit

All runs under the pinned Hermit used one flag set (`--chaos --max-timeslice=100000 --skid-margin=25
--panic-on-rbc-overshoot --epoch=2026-01-01T00:00:00Z --base-env=minimal`, one seed per run) with `-n 8 -tick-ms 100
-q 10`, run as root, one run at a time on one core. Build note: glibc built with -U_FORTIFY_SOURCE added to CFLAGS (upstream's default no-fortify build; same effect as upstream fix 127cefd, BZ 31928), Ubuntu gcc 13.3.0 defaults otherwise.

P1 is: U plus one hunk, a 10 ms clock_nanosleep between the CAS loop and the `__g1_start` load; positive control; assisted, window_widening, library_patched.

- An informational cell in the planted-build job (2026-10-17, about 21:15), never graded: 30 runs of P1's binary at
  `-r 60` all completed without the property firing.
- A probe (2026-10-17, 22:27 to 22:33), recorded only: at `-r 125` the property site was false in 0 of 30 runs (wall
  time 2,013 to 2,566 ms, no failed run); at `-r 500` in 0 of 30 runs (wall time 7,401 to 8,196 ms, no failed run).

The rule announced before that probe: at least one hit in the 60 runs and P1 stays; zero, and the positive control needs
a new design under its own specification.

[I] Why P1 may not have produced the state, as a source-based hypothesis only: the sleep runs in every waiter that
consumes a signal, for the same fixed time. Under Hermit, equal sleeps wake strictly in deadline order, so a waiter
signalled later wakes later and does not pass the earlier one through the window; the two group rotations the bug needs
would have to come from threads that are not themselves held in the window. This was the reasoning behind the second
design; it was not tested separately.

### 5.4 The second positive control

P2 is: U plus one hunk (blob `e58d3b83`), a 40,000-iteration PAUSE busy-wait between the signal-grab CAS and the `__g1_start` load; positive control; assisted, window_widening, library_patched.

Build note for both P2 builds: glibc built with -U_FORTIFY_SOURCE added to CFLAGS (upstream's default no-fortify build; same effect as upstream fix 127cefd, BZ 31928), Ubuntu gcc 13.3.0 defaults otherwise.

[I] The idea: in chaos mode a timeslice ends after at most 10,000 retired conditional branches at the flag set used here,
and at each slice end the thread draws a new random priority. A spin of at least 10,000 conditional branches therefore
contains at least one slice end per pass, so every waiter in the window is held for an independently drawn time instead
of a fixed one, and a waiter signalled later can overtake an earlier one. The thread stays runnable; the hunk makes no
system call and touches no shared memory; a `"memory"` clobber keeps the compiler from moving the `__g1_start` load
above the loop.

Container gate of the first P2 build (2026-10-17, 23:47; that build was U plus one hunk (blob `6ae599f9`), a 10,000-iteration PAUSE busy-wait between the signal-grab CAS and the `__g1_start` load; positive control; assisted, window_widening, library_patched; superseded, never run under Hermit): the build passed its checks, and the
disassembly showed the loop unrolled by two (`pause; pause; sub $0x2; jne`), so each pass had 5,000 conditional branches,
half the designed quantity. The count was changed to 40,000 before any run under Hermit (section 4).

Container gate of the corrected build (2026-10-18, 01:08 to 02:05): after stripping debug information, U's and P2's
`libc.a` differ in exactly `pthread_cond_wait.o` (1 of 2,204 members); in each of the 3 inlined copies of the loop the
compare-and-swap precedes the loop, the `__g1_start` load follows it, the loop constant is 40,000 and the step 2, giving
20,000 conditional branches per pass; the system-call, lock-prefix, fence and compare-and-swap counts equal U's.

Build on the test machine (2026-10-18, from about 04:24): after stripping debug information, P2's `libc.a` and U's
differ in one member, `pthread_cond_wait.o` (2,204 members compared); the disassembly gate counted 3 pause-loop copies,
each with 20,000 conditional branches per pass and no `__g1_start` load hoisted above the loop; the counted and
violation-only workload builds and the timing probe linked statically against P2's own sysroot with no system glibc
component; a second build at the same path was identical.

Probe, same job: under the pinned Hermit, P2's binary had the property site false in 0 of 30 runs at `-r 125` and in
0 of 30 runs at `-r 250`, with 0 failed runs and 0 runs with a Hermit failure note.

### 5.5 Determinism, timing and detector gates under Hermit

These gates do not depend on a positive control and were run independently of the P2 work. The round count was to be
frozen by a mechanical timing rule before any counted run: the median wall time per run of each of U and C had to be at
most 2.7 s (derived from the planned size of the counted run), halving the rounds along 500, 250, 125, 60, 30 until it
fit.

- At 500 rounds (2026-10-18, about 02:11) the medians were 7,045.5 ms (U) and 7,336.0 ms (C); the rule stopped and the
  next row of the table was launched.
- At 250 rounds (about 04:07): 3,754.0 ms and 3,831.0 ms; stopped again.
- At 125 rounds (about 04:15) every gate passed and 125 rounds were frozen, for counted runs that were never
  started.

At 125 rounds, labels bug_known_in_advance, targeted_workload, structure_from_public_reproducer, environment_pinned,
U unassisted, T2_instrumented; the planted binaries are detector self-tests only. Build note: glibc built with -U_FORTIFY_SOURCE added to CFLAGS (upstream's default no-fortify build; same effect as upstream fix 127cefd, BZ 31928), Ubuntu gcc 13.3.0 defaults otherwise.

- **Fast-forward recheck:** two 60-second virtual sleeps on arm C each completed in under 100 ms of wall time with at
  least 60 s of virtual time.
- **Determinism:** for each of 10 seeds, 3 runs of U gave identical standard output and identical schedule fingerprints
  (a hash of five fields of Hermit's run summary), and the same held for C; the 10 U seeds gave 10 distinct
  fingerprints. Here "deterministic" means exactly this check, nothing more.
- **Coverage and margin:** the workload produced contended signals and waits spanning two signals in all 30 U runs;
  the longest stretch of stagnant ticks in any healthy run was 0 for both arms (limit 3), with means of 7.70 (U) and 7.60
  (C) ticks per run.
- **Planted deaf condition variable:** applied in 30 of 30 runs, the property fired in all 30, the slowest run took
  1,239 ms of wall time.
- **Planted stalls:** leak, barrier and holdg each applied in 10 of 10 runs and ended as stalls of subtype L, B and M
  respectively, with the property true.
- **Recorded, not gates:** with the SDK build against the violation-only build of U on 10 seeds, the fingerprints were
  equal in 0 of 10 (the expectation), and the system-call differences ranged from -222 to 228; the design's formula of 33
  was a hypothesis and did not hold. This is measured support for the `T2_instrumented` label.

No Hermit failure occurred in any of the three launches.

### 5.6 The stop

The second positive control produced no hit in its 60 probe runs. By the rule recorded before the probe, there is no
third design, the positive-control gate was never run, and so the counted runs on U and C were never specified or
started.

## 6. What can and cannot be concluded

**Can be said**, each for the stated configuration only, labels as in 5.2. Build note: glibc built with -U_FORTIFY_SOURCE added to CFLAGS (upstream's default no-fortify build; same effect as upstream fix 127cefd, BZ 31928), Ubuntu gcc 13.3.0 defaults otherwise.

- A specific glibc source state can be built from source, pinned by hash, and shown by link trace, ELF checks and a
  runtime version line to be the only C library in a static test binary.
- On one machine, natively, the workload's property site was false at least once in 1,025 of 2,029 runs on the
  unmodified source state and in 0 of 2,029 on the fixed source state, and every one of the 1,025 printed end states had
  the pre-registered shape. This is a native observation with an arm-level control, on a heuristic detector.
- Under the pinned Hermit, the workload gave identical output and schedule fingerprints across three runs per seed for
  10 seeds per arm, the detector fired on every planted deaf condition variable and classified every planted protocol
  deadlock as a stall, not as a lost wakeup.
- Two library hunks that widen the race window, one a sleep and one a spin, did not make the property false in any of
  the runs made under the pinned Hermit (P1: 90 runs including 30 informational; P2: 60), nor P1 in its 200 native runs.

**Cannot be said:**

- Anything about whether Hermit reaches the bug's state on the unmodified library, at any frequency. The counted runs
  were not done.
- Any bounded null for the unmodified library under Hermit. A bounded null required the positive-control gate.
- Any comparison of a native figure with a figure under Hermit, or of either positive control with U or C.
- Any rate as a portable fact: every number belongs to one machine, kernel, compiler, build and run.
- Any statement that the two positive-control hunks, or Hermit, are incapable of producing the state: only what was
  observed in these runs is reported. [I] Sixty probe runs can only bound a per-run rate loosely, and the source-based
  reasons in 5.3 and 5.4 are hypotheses.
- Anything about distribution glibc packages, first affected versions, or other machines.

## 7. Repeating each step

The scripts that ran, with only machine paths replaced and a list of every change against the versions that ran, are in
[`examples/glibc-condlock/experiment`](../examples/glibc-condlock/experiment), together with both positive-control
patches and the Hermit build recipe.

1. **Sources.** Fetch `release/2.40/master` from the glibc repository; check out `d8a1a1ae` (U) and `8d3dd23e` (C);
   apply the P1 or P2 patch to U for the positive controls; check the file count (19,756) and that only
   `nptl/pthread_cond_wait.c` differs between U and P.
2. **Tools.** Build GNU bison 3.8.2 and m4 1.4.19 from GPG-verified tarballs into a private prefix.
3. **glibc.** Configure, build and install each tree into its own sysroot with the recipe in section 3. Build note: glibc built with -U_FORTIFY_SOURCE added to CFLAGS (upstream's default no-fortify build; same effect as upstream fix 127cefd, BZ 31928), Ubuntu gcc 13.3.0 defaults otherwise.
4. **Workload.** Build the C SDK and `condlock` statically against each sysroot (`-DCONDLOCK_ARM_U` for U and the
   positive controls, `-DCONDLOCK_ARM_C` for C); check the link trace, the ELF checks and the `libc=2.40 hdr=2.40` line.
5. **Native reference.** Run U and C alternately with `-n 8 -r 5000 -tick-ms 50 -q 10`; classify each run from its exit
   status and its own report (exit 3 with the property site false is a hit).
6. **Under Hermit.** Build Hermit with the recipe in the experiment folder, use the flag set of 5.3, one seed per run,
   `-n 8 -r 125 -tick-ms 100 -q 10`, a 120 s cap per run; compare three runs per seed.
7. **Positive-control probe.** Run the P2 binary for seeds 319891 to 319920 at `-r 125` and 319921 to 319950 at
   `-r 250`.

Before running the drivers on any machine, read "Security notes for running the drivers" in the experiment folder's
README: the Hermit calls run as root through `sudo -n`, the drivers create world-writable (mode 1777) directories, and
they were run on a dedicated machine.

What you cannot repeat exactly: our Hermit binary ([I] a rebuild will probably have a different sha256, so seeds,
fingerprints and timings are not comparable), our binary hashes (the build path enters the debug information; compare
stripped objects), and our native counts (they belong to our machine). The scripts' reuse gates compare every input with
the hash recorded in our run, so they stop on your builds until you replace those pins with your own. Hermit was run
as root; an unprivileged run was not tested. The raw run records are not published with this page.

### What the scripts print

The evaluators end with one outcome line. The codes you will see, in reader terms:

| Printed code | Meaning |
|---|---|
| `PASS`, `PASS+CL-FIRED-U` | build and isolation checks passed; the suffix says the U plain-run cell recorded hit runs, which pauses the native step for a human decision |
| `N-DONE/NU-HIT` | native reference completed; U had at least one hit run; C had none |
| `X1C-PASS+CTRL-DIFFER-UCP` | planted builds built, pinned and byte-identical on rebuild; the suffix says the rebuilt U, C and P1 workload binaries differ from the earlier build (equal once debug information and build ID are stripped) |
| `X1D-RAN`, `PX-RAN` | a probe ran all its runs; nothing is graded |
| `X1E-PASS` | P2 built, disassembly gate passed, binaries pinned, smoke passed, probe ran all its runs (nothing graded); this run's line also shows `b2=REPRO` (rebuild identical) |
| `GLG-A-STOP/G0T-LOWER-R` | determinism gates stopped on the timing rule; halve the rounds and launch the next row |
| `GLG-A-PASS` | every gate under Hermit passed and the round count is frozen |

## 8. Acknowledgements

The bug report by Qin Li; the analyses of Malte Skarupke and Torvald Riegel; the nine fix commits by Frank Barrus and
Malte Skarupke, each reviewed by Carlos O'Donell; and, separately, the initializer-compatibility commit by Florian
Weimer.

## Appendix. Labels used

| Label | Meaning |
|---|---|
| native reference | measured without Hermit; never a claim about it |
| bug_known_in_advance | the bug, its fix and its public reproducer were known before any run |
| targeted_workload | the workload was written to exercise this bug |
| structure_from_public_reproducer | the workload's structure follows a public reproducer; no code copied |
| environment_pinned | the number belongs to the stated machine, kernel, compiler, build and run |
| T2_instrumented | built with the reporting SDK, which changes the schedule compared with an unreported build |
| unassisted | the library is unmodified upstream source |
| fixed_branch_control | the fixed source state, compared at arm level only, never seed for seed |
| assisted, window_widening, library_patched | a hunk added by hand to the library to widen the race window |
| planted | a detector self-test build with an injected fault; never a glibc result |
