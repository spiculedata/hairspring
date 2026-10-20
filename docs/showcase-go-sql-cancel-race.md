# Counting a database/sql cancel race under a seeded scheduler: 6,000 runs on three Go toolchains

This page describes an experiment on one known Go bug, CVE-2025-47907 (GO-2025-3849): a race between cancelling a query
and `Rows.Scan` in `database/sql`, fixed in Go 1.24.6. We wrote a small workload around a fake driver, ran it under
Hermit, the deterministic scheduler behind Hairspring's `process` backend, on three toolchains (Go 1.24.5, Go 1.24.6, and
Go 1.24.5 with only the fix applied), and counted. Each toolchain ran the same 2,000 seeds, 6,000 runs in all, on one
machine.

What you can take from it:

- how to build one Go program three ways, with the official releases and a patched toolchain that differs from stock in
  exactly the fix's two files, pinned by hash, and use the unpatched-versus-fixed pair as a control;
- a stuck-state style property for a pooled-connection race, designed so that it cannot be false on a correct library,
  plus a deliberately assisted version of the workload that checks the detector fires;
- a count, under the skid-b reading of the outcome (mechanical code `C1-DONE/R-CRASH+R-CRASH-hermit(u=2,c=0,cp=0)`; both
  codes are given in 5.5): in this configuration the property site was false at least once in 934 of the 1,996 rated runs on
  Go 1.24.5, and in none of 2,000 runs on each of the two controls (labels in 6.2);
- under the same reading, confirmation of the first 100 of those hit runs, each three times by direct rerun, once by
  Hairspring's engine at the same seed (a different Hermit environment), and once by replaying the engine's bundle (codes
  in 6.4);
- an account of the one place where the pre-registered outcome table and our reading of it disagree: Hermit itself
  stopped in four of the unpatched runs, and the evaluator prints two outcome codes, which differ only in how two of those
  runs (the two "shape B" failures) are read (5.5, 6.5).

What it does not have: any statement about other drivers, other Go versions, other machines, or why the interleavings
happened. A hit here is a counted run, not a defect report, and the number is not a rate of anything outside this setup.

The scripts and the Hermit build recipe are in
[`examples/go-sql-cancel/experiment`](../examples/go-sql-cancel/experiment); the workload and the pinned toolchain
scripts are in [`examples/go-sql-cancel`](../examples/go-sql-cancel).

## 1. The bug

`database/sql` copies a column value out of the driver's row into your destination in `Rows.Scan`. In Go 1.24.5,
`Scan` takes the rows' read lock, checks the rows' state, and releases the lock; only then does it copy the driver-owned
bytes (`lastcols`) into the destination, unless the destination is a `*sql.RawBytes`, in which case the lock stays held.
A context cancel runs `Rows.close` on another goroutine: it takes the write lock, closes the driver's rows and returns the
connection to the pool, where a waiting query can be handed it. The driver documentation says the data returned by `Next`
is valid only until the next `Close` or `Next`. So if the cancel lands after `Scan` released the lock and before the copy
finished, a second query can run the driver's `Next` on the same connection while the first `Scan` is still reading. If
the driver reuses one buffer for every row, the first `Scan` can return the second query's row or a mixture.

The advisory (GO-2025-3849) describes it as: cancelling a query during a call to `Scan` can, if other queries are made in
parallel, overwrite the expected results with those of another query, so `Scan` returns either unexpected results from the
other query or an error. It lists Go versions before 1.23.12 and 1.24.0 to before 1.24.6 as affected, and the fix is in Go
1.23.12 and 1.24.6 (golang/go commit `83b4a5db2409` on the 1.24 branch, "database/sql: avoid closing Rows while scan is
in progress"). The fix makes `Scan` hold the read lock across the whole copy.

Go 1.24.6 contains six other commits beside the fix, among them changes to `src/runtime` files (`malloc.go`, `mbitmap.go`,
`proc.go`, `runtime2.go`). That is why the experiment has a second control that changes nothing but the fix.

## 2. The workload and its property

### 2.1 What the workload does

The workload (`sqlcancel`) is new code written for this experiment, standard library only.

- **A fake driver** (`retaindrv`) with no I/O, timers or randomness. Its connection owns one 25-byte buffer that every
  row of every query on that connection reuses. A row is `q<id>-r<row>-q<id>`: the query's id appears twice, so a mixture
  of two queries' rows is visible as a mismatch. `Next` writes the row of its own query into the buffer and hands out a
  slice of it, which the driver contract allows. `Close` does not touch the buffer.
- **A pool of one connection** (`SetMaxOpenConns(1)`), so a released connection goes straight to a waiting query.
- **Four workers**, each issuing queries in turn, with four cancellers. Per query: the worker starts it with a cancelable
  context, calls `Next`, hands the cancel function to its canceller over an unbuffered channel without waiting for an
  acknowledgement, and calls `Scan` into a `*string`. The worker and the canceller then genuinely race.
- Flags: `-mode=std` (a `*string` destination, the unassisted workload) or `-mode=yield` (the assisted version, 2.3) and
  `-q` queries per worker (250 in every counted run).
- Standard output is one fixed-width line of counters. Exit status 0 on completion whatever the property result, 3 if a
  harness invariant failed, 64 on a usage error; a Go crash exits 2.

The workload reports through a small Go shim for Hairspring's guest-event protocol
([docs/guest-protocol.md](guest-protocol.md)), which makes it `T2_instrumented` (6.1). No engine feature produced any
counted number on this page: the 6,000 runs were direct Hermit runs driven by the scripts in
`examples/go-sql-cancel/experiment`, and hits were counted from each run's own report. The engine was used only for the
100 confirmations (6.4).

### 2.2 The property

`go.sql.scan_returns_own_row` is an "always" site, evaluated after every `Scan` that returned no error: the string equals
the row of its own query, exactly. A `Scan` error after a cancel is allowed (the advisory allows it) and is only counted.
When the site is false, a witness records the query id, the id at the head and at the tail of the returned string, and a
class: 1 foreign (head and tail equal, a complete row of another query), 2 torn (head and tail differ), 3 unparsable.

The property cannot be false on a correct library: the driver only ever writes complete rows of the query that owns the
connection at that instant, and a copy that finishes before the connection can change owner sees exactly its own row. Six
more sites are coverage markers, a mechanism marker or harness invariants (one connection owner at a time; an error only
after a cancel; every query ran). Only the property decides anything.

A **hit** in this page is a run whose own protocol report shows the property site false at least once. Hits are counted per
site from that report, never from a tool's pass/fail verdict. Classes are assigned afterwards by the evaluator, with this
precedence: timeout (inconclusive, never a hit), Hermit failure, Go crash, harness failure, hit.

### 2.3 The assisted control

`-mode=yield` scans into a `sql.Scanner` that copies half of the value, sleeps 1 ms, and copies the rest. That widens the
window on purpose, in the same structure as the upstream regression test (a Scanner that blocks mid-`Scan`). It checks
that the property and the detector fire; it says nothing about how easily the unassisted race is reached. Its labels are
`assisted` and `window_widening`, and we do not call its hits "reproduced".

### 2.4 The pinned Hermit

Hermit runs a Linux x86-64 program one thread at a time under a seeded scheduler with virtual time. In chaos mode it ends
timeslices at random points, measured in retired conditional branches, and gives threads random priorities. The build used
here is pinned by its sha256 (`666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a`, `hermit 0.4.1
(2026-10-15)`): commit `25781c5` of the maintained fork with two replay commits cherry-picked on top. The recipe is in the experiment
folder. Every number below that was measured under Hermit
belongs to that build; [I] a different build may give different schedules for the same seed.

## 3. Arms, builds and labels

| Arm | Toolchain | Role and labels |
|---|---|---|
| A-u | official go1.24.5 | the claim arm; `unassisted` (`-mode=std`), `T2_instrumented`, `bug_known_in_advance`, `targeted_workload` |
| A-c | official go1.24.6 | `fixed_release_control`, at arm level only (six other commits, see 1); `bug_known_in_advance`, `targeted_workload` |
| A-c' | go1.24.5 built from source with only `patches/fix-only.patch` (the two non-test files of the fix) | `patched_toolchain`, the narrow control; `T2_instrumented`, `bug_known_in_advance`, `targeted_workload` |
| P-u, P-c, P-c' | the same three | the assisted control only: `assisted`, `window_widening`, `T2_instrumented`, `bug_known_in_advance`, `targeted_workload` |

Every counted binary is a static linux/amd64 build with `CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GOAMD64=v1 go build
-trimpath -buildvcs=false -ldflags=-buildid=`, built twice with equal sha256 on the build machine (an Apple-silicon
workstation, bootstrapped from the official go1.24.5 darwin-arm64 release), with these identities:

| Arm | Binary | sha256 |
|---|---|---|
| A-u | `u-shim` | `282fddca507d718fa255c3fa35c3ab6ec9d98c217f839deb80d8e48c1d6da822` |
| A-c | `c-shim` | `6ba4c482af014bcf00b7dc175d4ed75aabc396e402a40f1a1a406910b20ca76d` |
| A-c' | `cp-shim` | `592d8435fd44f774bc18e0881b55c69dc779844c02be27603217fb3b3bc81341` |

The fix-only patch has sha256 `5e9603cf389584c39e11f26d1f7bcc04f4cd38c6d548303b95de23e1ed1b00d2`, and
`scripts/check-patched-diff.sh` in the example checks that the patched tree differs from the stock 1.24.5 tree in
exactly `sql.go` and `convert.go`. The binaries rebuild from `examples/go-sql-cancel` at the repository commit of this page (no tag), on darwin-arm64
with the toolchains of `scripts/fetch-go.sh` (the recorded sha256 values were built there).

**`T2_instrumented`:** built with the protocol shim; the schedule differs from an unreported build (fingerprints differ on
10 of 10 seeds, syscall counts by -156 to 2,201 in one run each); detection agrees with the unreported build (the
comparison of 6.1).

Nothing on this page is a statement about any Go program other than this workload, any driver other than this fake one,
or any Go version other than the three above.

## 4. The machine and the run configuration

Every Hermit run on this page used one machine: Ubuntu 24.04.5, kernel 6.17.0-42-generic, x86-64 (Intel Xeon E5-1410 v2).
Each run was `hermit run` as root through `sudo -n` (runs as an unprivileged user stopped at the user-namespace uid mapping
in all ten probes we made), with the Hermit process environment reduced to `PATH`, `HOME` and `LANG`, `nice -n 10`, on
cores 1 to 3, one run at a time, a 60 s cap per run, and these flags:

```
--epoch=2026-01-01T00:00:00Z --base-env=minimal --env=GOMAXPROCS=2 --env=GOTRACEBACK=all
--chaos --max-timeslice=100000 --skid-margin=25 --panic-on-rbc-overshoot --sched-seed=<seed>
```

followed by the binary and `-mode=std -q 250`. In a probe, a summary-file directory owned by the unprivileged user at mode
775 made the run as root stop with a permission-denied panic, while mode 1777 (or a root-owned directory) did not; the
counted runs wrote their summaries into mode-1777 directories.

## 5. How the experiment was designed and pre-registered

Each step was a separate specification with its scripts embedded and hashed. An independent review step signed each
experiment before it ran. Each specification fixed, before any of its data existed: the commands, the seeds, the evaluator
code, the outcome table (the first rule that holds gives the outcome), and the sentences each outcome allows. The
evaluator, not a person, printed the outcome; results were copied, not re-judged.

### 5.1 The seeds and the arms

Seeds 330001 to 332000, the same list in all three arms (the seed of index `i` is `330000 + i`), so 6,000 counted runs.
A *block* is one seed run in the three arms, with the starting arm rotating from seed to seed, so every arm has the same
number of runs and the same positions. There are no spare or replacement seeds, and a seed that has run is never run again.
The number of seeds was fixed before any counted run; no early stopping.

### 5.2 Windows

The counted runs were run in five windows of 400 seeds (1,200 runs each), then the confirmations in two more windows.
Each later window started only after the earlier one was archived and its code read. Nothing stops a window on a result:
the halts are infrastructure only (for example, the first 15 runs of a window all failing).

### 5.3 The outcome table in one paragraph

The counted runs end in one outcome code. In short: any hit in a control arm, any harness failure, any Go crash or any
unrecognised Hermit failure blocks claims; otherwise a complete run with at least one hit in the unpatched arm needs the
hits to be confirmed. The rate comparison with the controls needs the lower Wilson bound of the unpatched arm to exceed the
larger upper Wilson bound of the two controls, which at 2,000 runs per arm means at least 8 hits. The confirmation sample
is the first 100 hit seeds in seed order (a cap set by how long the confirmations take, not a statistical choice); the
count and the interval come from all the counted runs.

### 5.4 Departures and later additions, stated plainly

- The 100-seed confirmation cap was fixed before the first counted run, with a budget-only reason.
- `--verify` was not run on the hits (a departure from the design note): Hermit's `--verify` uses a lossy comparison, and
  it was judged to add little over identical output and fingerprints across three direct reruns plus the bundle replay.
- The reading of Hermit failures of "shape B" (5.5) was fixed after window 1 had produced one, so it is not
  pre-registered in the strict sense; this is why the mechanical code is stated beside every result.

### 5.5 Hermit failures, and the two readings

Hermit has an internal consistency check on its branch counter (a "skid overshoot"). When it trips, the run stops with
exit status 125 and a Hermit failure message. Before the counted runs it was pre-registered that such a run is not a
rated run, is not replaced, does not change the seed list, and is listed per arm as a ledger.

In window 1 one run (`C1.u.i0172`) stopped on the same trigger but a different path: the same overshoot message and panic
at the same source line, followed by Hermit's "cleanup remains unconfirmed" path instead of a task exit. We call this
"shape B". It was not part of the pre-registered ledger, so the pre-registered table treats it as an unrecognised Hermit
failure that blocks claims. After window 1, a rule was fixed that selects shape B by its stderr signature and reads it as
the same ledger, only for a window whose leftover-process scan afterwards was clean; the rule is applied mechanically to
every run, not run by run. A second shape-B run occurred in window 2. The evaluator therefore prints two codes:

- the **mechanical code** (`OUTCOME`), which treats shape B as a blocking Hermit failure:
  `OUTCOME C1-DONE/R-CRASH+R-CRASH-hermit(u=2,c=0,cp=0)`;
- the **skid-b reading** (`OUTCOME-SKID-B`), which reads shape B as the ledger:
  `OUTCOME-SKID-B C1-DONE/R-HIT+R-CRASH-hermit(u=4,c=0,cp=0)`.

The two codes come apart only because of the two shape-B runs (`C1.u.i0172`, `C1.u.i0458`). The other two failures
(`C1.u.i0284`, `C1.u.i1711`) carry the pre-registered signature and are in the ledger under both codes; that is why the
mechanical suffix counts two and the skid-b suffix four.

The two codes differ only in how two runs are read, and the evaluator's tables (6.2) are printed identically under both. The
sentences built on them (the counts in words, the comparison with the controls in 6.3, the confirmation in 6.4) are made
under the skid-b reading; under the mechanical code only the tables and the failing runs in full may be stated. We state
the mechanical code beside every result below.

## 6. Each step and its result

All results are for one machine and one run of each configuration, on the dates given (UTC).

### 6.1 Checks before the counted runs

- **Go under Hermit** (2026-10-16): a small Go program with a goroutine ring and one SQL round trip ran under the pinned
  Hermit: for seeds 339001 to 339010, three runs each gave identical output and schedule fingerprint, and 20 seeds gave 20
  distinct fingerprints. Hairspring's engine, run as root, passed 20 trials of the program, failed a second target of
  the program at seed 339001, and replayed the bundle with output and schedule matching the recording.
- **Determinism** (2026-10-17, seeds 339521 to 339530, `-q 250`): for each of the three arms, three runs of each of ten
  seeds gave identical standard output and identical schedule fingerprints (a hash of five fields of Hermit's run
  summary). Here "deterministic" means exactly this check, nothing more.
- **The assisted control**, labels as in section 3: P-u had the property site false in 199 of 199 rated runs (Wilson 95%
  [98.11%, 100%]; one further run was a Hermit failure and is outside the rated runs); P-c and P-c' in 0 of 200 each
  (exact one-sided upper bound 1.4867%).
- **Does the protocol shim change detection?** The violation-only build (no event stream) and the shim build agreed on the
  assisted control: 199 of 199 against 199 of 199 on the unpatched toolchain and 0 of 200 against 0 of 200 on the patched
  one. Across 10 seeds the fingerprints of the two builds differed on all 10, and the syscall counts differed by -156 to
  2,201 in one run each.

### 6.2 The counted runs: arm A-u and the controls

Labels: bug_known_in_advance; targeted_workload; environment_pinned; A-u unassisted and T2_instrumented; A-c
fixed_release_control at arm level only; A-c' patched_toolchain and T2_instrumented. Configuration: section 4, binaries
of section 3, `-mode=std -q 250`, seeds 330001 to 332000 in every arm. The five counted windows ran from 2026-10-17 23:44
to 2026-10-18 about 10:00; each ended at its planned size, and no binary changed during any window.

| Arm | Decided | Rated | Runs with the property site false at least once | Wilson 95% | Not rated |
|---|---|---|---|---|---|
| A-u (go1.24.5) | 2,000 | 1,996 | **934** | [44.61%, 48.99%] | 4 Hermit failures (section 6.5); no timeout, Go crash or harness failure |
| A-c (go1.24.6) | 2,000 | 2,000 | 0 (exact one-sided upper bound 0.1497%) | [0.00%, 0.19%] | none |
| A-c' (go1.24.5 plus the fix) | 2,000 | 2,000 | 0 (exact one-sided upper bound 0.1497%) | [0.00%, 0.19%] | none |

Under the skid-b reading, in this configuration, A-u had 934 of 1,996 rated seeds with at least one false evaluation of the
property, Wilson 95% [44.6129%, 48.9866%]; A-c and A-c' had 0 of 2,000 each. No timeout, Go crash or harness failure
occurred in any arm. This is a count of runs on one machine with our test driver, not a rate of defects.

Codes: mechanical `C1-DONE/R-CRASH+R-CRASH-hermit(u=2,c=0,cp=0)`; skid-b reading `C1-DONE/R-HIT+R-CRASH-hermit(u=4,c=0,cp=0)`.
The table above is printed identically under both; the sentence under it is made under the skid-b reading.

The median wall time per run was 4,202 ms (A-u), 4,228 ms (A-c) and 4,215.5 ms (A-c'), context only; no speed claim is
made.

### 6.3 The comparison with the controls

Under the skid-b reading, with A-c and A-c' both at 0 of 2,000, the lower Wilson bound of A-u (44.6129%) is above the
larger upper Wilson bound of the two controls (0.1917%), and the 8-hit threshold of the design is passed by a wide margin.
The three rates side by side, with the assisted control for context: A-u 934 of 1,996 [44.61%, 48.99%]; A-c 0
of 2,000 [0.00%, 0.19%]; A-c' 0 of 2,000 [0.00%, 0.19%]; assisted control P-u 199 of 199 and P-c, P-c' 0 of 200 each
(section 6.1; assisted, window_widening). The controls are a fixed release (arm level only) and a patched toolchain; the
comparison is not "the same seed passes on 1.24.6". The sensitivity line (information, not a rule): had every unrated A-u
cell been a hit, the count would be at most 938 of 2,000.

Codes: mechanical `C1-DONE/R-CRASH+R-CRASH-hermit(u=2,c=0,cp=0)` (the comparison is not stated under this code); skid-b
reading `C1-DONE/R-HIT+R-CRASH-hermit(u=4,c=0,cp=0)`.

### 6.4 Confirmation of the first 100 hits

The hit seeds in seed order begin 330003, 330004, 330008; the first 100 end at 330228, all within window 1. For each of the
100:

- **Direct reruns** (2026-10-19, same wrapper, binary, flags and seed): three more runs, all three a hit, each with the same
  standard output hash and schedule fingerprint as the original run: 300 of 300 rerun rows were hits.
- **The engine** (2026-10-19): one `hairspring test` of the A-u binary at that seed, and, with exactly one bundle written
  each time, one `hairspring replay`. The engine reported a failure at the hit's own seed (its own message ends "reproduced
  3/3"), for all 100;
  the replay of its bundle matched the recording's output and schedule and the recorded property failed again, for all 100.
  The engine ran Hermit in a different environment from the direct driver (its own environment variables, and its three
  trials at seeds S, S+1 and S+2 in parallel); this is an observation about that environment, not a repetition of the
  direct run.
- **The first false evaluation** of each of the 100 was of class 1 (foreign): a complete row belonging to another query.

The other 834 hits are counted and listed, and were not confirmed. Under the skid-b reading, the statement is: the property
site was false in 934 of 2,000 blind seeds (1,996 rated; Wilson 95% [44.6129%, 48.9866%]); the first m = 100 of them in
seed order were each confirmed (3/3 identical direct reruns, the engine at the same seed, bundle replay), on Go 1.24.5, with
our test driver, bug known in advance, `T2_instrumented`; the engine ran in a different Hermit environment. No rate
comparison with the controls is part of this sentence (see 6.3). The confirmation of a hit seed is an observation of this
setup; it says nothing about why the interleaving happened.

Codes: mechanical `OUTCOME C1-DONE/R-CRASH+R-CRASH-hermit(u=2,c=0,cp=0)` (the confirmation rows are recorded; no claim is
made under this code); skid-b reading `OUTCOME-SKID-B C1-DONE/R-HIT+R-CRASH-hermit(u=4,c=0,cp=0)`.

### 6.5 The Hermit failures

Four A-u runs ended in a Hermit failure with exit status 125 and were not rated or replaced: `C1.u.i0284` (window 1) and
`C1.u.i1711` (window 5) with the pre-registered skid signature, and `C1.u.i0172` (window 1) and `C1.u.i0458` (window 2)
with the shape-B signature of 5.5. The skid-b reading was available for those two because the leftover scan after
windows 1 and 2 was clean (no Hermit process, mount, control group or shared-memory entry). A-c and A-c' had no Hermit
failure. Hermit failures are outside the rated runs; nothing here says they are harmless, rare in general or unrelated to
the workload, and the sensitivity line above shows how far they could move a count.

## 7. What can and cannot be concluded

**Can be said**, each for the stated configuration only, labels as in 6.2:

- Under the skid-b reading, on one machine, with our fake driver, under the pinned Hermit, the property site was false at
  least once in 934 of 1,996 rated seeds of the unmodified Go 1.24.5 workload (Wilson 95% [44.6129%, 48.9866%]), and in 0 of
  2,000 on each of Go 1.24.6 and Go 1.24.5 with only the fix.
- Under the skid-b reading, the first 100 hit seeds in seed order were each confirmed (3/3 identical direct reruns, the
  engine at the same seed in a different Hermit environment, bundle replay).
- Under the skid-b reading, the lower bound of the A-u interval is above the upper bounds of both controls.
- Under the mechanical code only the tables of 6.2 and the failing runs of 6.5 stand; the sentences above do not.

**Cannot be said:**

- Anything about a rate of defects, a cause of any hit, or why a seed hits.
- Any statement about other Go versions, other drivers (including real ones), `Row.Scan`, `Tx`, `Conn`, `Stmt`, `RawBytes`
  or other hosts, kernels or `GOMAXPROCS` values.
- That Hermit reaches the race better than something else, or that a hit would appear natively; no native figure is
  compared with a figure under Hermit.
- That the 934 hits are 934 distinct interleavings.
- That the 834 unconfirmed hits would each be confirmed.
- That Hermit failures are rare or harmless in general.
- Anything about the speed of any run.

## 8. Repeating each step

The scripts that ran, with machine paths replaced, are in
[`examples/go-sql-cancel/experiment`](../examples/go-sql-cancel/experiment), with the Hermit recipe.

1. **Toolchains.** `scripts/fetch-go.sh`, then `scripts/build-patched-toolchain.sh` and `scripts/check-patched-diff.sh` in
   the example. The first two work on macOS arm64 only (they use the `darwin-arm64` archives, which is where the recorded
   binary sha256 values were built). On Linux, download the `linux-amd64` archives pinned in the example's `PINS` by hand.
2. **Workload binaries.** Build `cmd/sqlcancel` with each toolchain and the flags of section 3; compare the sha256 values.
3. **Hermit.** Build it with the recipe in the experiment folder (root is needed to run it).
4. **Counted runs.** Use the five windows of 400 seeds from 330001 with `c1.sh count`, and `c1_eval.py verdict`.
5. **Confirmation.** `c1.sh confirm` for the direct reruns and `c2e.sh` for the engine, on the first 100 hit seeds.

What you cannot repeat exactly: our Hermit binary ([I] a rebuild will probably have a different sha256, so seeds and
fingerprints are not comparable), our machine, and our raw run records (not published). The scripts' gates compare every
input with the hash recorded in our run, so they stop on your builds until you replace the pins.

### What the scripts print

| Printed code | Meaning |
|---|---|
| `C1-DONE/R-HIT+R-CRASH-hermit(u=4,c=0,cp=0)` (`OUTCOME-SKID-B` line) | all 2,000 seeds decided in every arm; the unpatched-arm count passes the pre-registered comparison; four Hermit failures in the unpatched arm, read as the ledger |
| `C1-DONE/R-CRASH+R-CRASH-hermit(u=2,c=0,cp=0)` (`OUTCOME` line) | the same tables; the two shape-B failures are read as blocking, which stops every claim beyond the rows themselves |
| `C1-DONE/C2-PENDING`, `C1-DONE/R-HIT-CANDIDATE` | counted runs complete; confirmation not yet complete. These two appeared only on the `OUTCOME-SKID-B` line (after window 5 and after the direct reruns); the mechanical `OUTCOME` line stayed `C1-DONE/R-CRASH+...` throughout |

## 9. Acknowledgements

The advisory credits Spike Curtis from Coder. The fix is Go commit `83b4a5db2409`.

## Appendix. Labels used

| Label | Meaning |
|---|---|
| bug_known_in_advance | the bug, its fix and the conditions that expose it were public before any run |
| targeted_workload | the workload was written to exercise this bug |
| environment_pinned | the number belongs to the stated machine, kernel, Hermit build, flags and run |
| T2_instrumented | built with the reporting shim, which changes the schedule compared with an unreported build |
| unassisted | the standard-library toolchain unmodified, a `*string` destination |
| fixed_release_control | the official release that contains the fix, compared at arm level only |
| patched_toolchain | the unpatched release plus only the fix's two files |
| assisted, window_widening | a Scanner that sleeps mid-copy to widen the race window; a positive control only |
