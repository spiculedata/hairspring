# Language fixtures: a planted lost-update race in C, C++, Go, Rust and Python
Small programs with the same planted race, each reporting through its language's Hairspring SDK
([`sdks/c`](../../sdks/c) for C and C++, [`sdks/go`](../../sdks/go) for Go, [`sdks/python`](../../sdks/python) for
Python; the Rust fixture sits beside its SDK in [`sdks/rust/fixtures`](../../sdks/rust/fixtures)). They are test subjects
for checking that a language's runtime, its SDK and the `process` backend work together; they are not examples of good
concurrent code.

The fixtures follow one output and mode contract for all five languages (C, C++, Go, Rust and Python), so that every
language's program can be run and judged the same way. This page is that contract; C, C++, Go and Python live here, and
the Rust program follows the same contract in its own directory.

## One program, three modes

Each language builds **one artifact** that takes its mode as its only argument. The mode is never a build flag, so one
sha256 identifies what ran in every arm.

| Artifact | Built by | Modes |
|---|---|---|
| `at-c` | `make -C c` | `buggy`, `fixed`, `wide` |
| `at-cxx-clang`, `at-cxx-gxx` | `make -C cpp` (clang++ or g++) | `buggy`, `fixed`, `wide` |
| `at-go` | `make -C go` | `buggy`, `fixed`, `wide` |
| `python/at_py.py` (with `python/probe_py.py` and the SDK package) | nothing to build; `scripts/stage-python.sh` stages and hashes it | `buggy`, `fixed`, `wide`, and `--switch=1.0` as an optional second argument |

- `buggy`: the planted race, no lock.
- `fixed`: the same program with a mutex held across the read-modify-write (`pthread_mutex_t`, `std::mutex` with
  `std::lock_guard`, `sync.Mutex`, `threading.Lock`); it cannot lose an update.
- `wide`: `buggy` with a busy loop ten times longer. Nothing else differs.

Any other argument, no argument, or a spare argument prints a usage line on stderr, prints nothing on stdout and exits 64.

## The program

Four workers each do 1,000 times:

1. read a shared counter;
2. run a busy loop of W rounds of user code (no system call, no allocation, no lock, no clock read);
3. write back the value read plus one.

The read and the write are separate relaxed atomic operations (sequentially consistent in Go), so the program has no
data race in the language's memory model, but an update can be lost when two workers read the same value.

Before its first iteration every worker increments a shared `started` counter and waits until all four have started
(a start barrier), so the four windows overlap from the first iteration.

| Fixed parameter | Value | Where |
|---|---|---|
| workers (T) | 4 | `LANG_AT_THREADS`, `race.Workers` |
| iterations per worker (K) | 1,000 | `LANG_AT_ITERS`, `race.Iterations` |
| busy-loop rounds in the window (W) | 1,000 | `LANG_AT_SPIN`, `race.Spin` |
| busy-loop rounds in `wide` (W x 10) | 10,000 | `LANG_AT_SPIN_WIDE`, `race.SpinWide` |
| expected final value | 4,000 | `LANG_AT_EXPECTED`, `race.Expected` |

W is the same number in every language, and the loop is the same multiply-add chain; the retired branches per round
differ by compiler and are not equalised.

The busy loop has to survive the optimiser. In C and C++ it is a function that is never inlined and whose loop holds an
empty `asm volatile` with a memory clobber in every round, which also stops the compiler from moving the counter's load
and store across it. In Go it is a `//go:noinline` function whose result is stored to a per-worker atomic after the
window. The native self-check below shows the window is real: a build whose loop had vanished would almost never lose
an update.

The memory orders differ between the languages. C and C++ use relaxed loads and stores. Go's atomics are sequentially
consistent, so the Go counter's store is a stronger operation (a locked exchange on x86). The window is the busy loop in
every language and the lost update does not depend on the order, but the programs are not instruction-for-instruction
equivalent and the native rates should not be compared across languages.

## What each program reports

- **Property** `<lang>.counter_exact` (`always`; `<lang>` is `c`, `cxx`, `go`, `rust` or `py`): the final counter equals 4,000.
  Evaluated once, in the main thread after the workers are joined, with four witness values `[final, 4000, 4, 1000]`.
- **Coverage marker** `<lang>.all_started` (`sometimes`): all four workers started. Evaluated once, in the main thread.
- **Work**: one operation per iteration (4,000 in total), counted after the store, outside the window.
- **Stdout**: exactly one line, written with one write call:
  `LANGAT v1 lang=<id> mode=<mode> final=<20 digits> expect=<20 digits> order=<16 hex> rand=<16 hex>`.
  `order` is FNV-1a over the arrival list written as four ASCII digits (the worker ids in the order they finished);
  `rand` is one value from the system's entropy source, so a run that leaks entropy into its output shows it.
- **Exit status**: 0 whatever the property says (the verdict is the protocol report, not the exit code); 3 for an
  internal invariant failure (a worker could not be created or did not finish, or the result line could not be
  written); 64 for a usage error. In Go the usage check happens before the SDK starts, so a usage error exits 64 without
  the SDK's report; the C and C++ programs exit 64 the same way and their SDK report shows no evaluations.
  The check script exercises the exit-3 path for an unwritable result line (stdout closed). A failed thread creation
  cannot be injected without hook code in the artifact, so that path is covered by review only.

The programs do not read the scheduler seed, `/proc`, the CPU count or the clock; `scripts/check-sources.sh` searches
the sources for them.

## Layout

```
common/lang_at.h       the parts C and C++ share: parameters, modes, busy loop, order hash, stdout line
c/                     src/lost_update.c, Makefile
cpp/                   src/lost_update.cc, Makefile (clang++ by default; CXX=g++ works)
go/                    main.go, internal/race (the program and its unit tests), Makefile
dotnet/                the BCL-only .NET probe: src, selftest, test_probe.sh, profiles.manifest, Makefile
tests/seccomp_window.c test-only: the window makes no system call (used by the build record, Linux only)
scripts/check-native.sh  the native self-check of one artifact; run-checks.sh adds the negative control;
                       check-sources.sh, check-disasm.sh; test-scripts.sh tests all of them with a fake program
scripts/build-record.sh  generates the build record of one artifact (build-record-docker.sh runs it in a container)
Makefile               make test: all three
```

## Build and test

```sh
make test                  # everything below
make -C c test CC=clang    # or CC=gcc
make -C cpp test CXX=clang++   # or CXX=g++
make -C go test            # needs Go 1.24 or newer
make -C go test-race       # the race detector, needs cgo
```

Each language's `make test` builds the artifact, then:

- searches the sources for the inputs the programs must not read;
- checks the arguments: no argument, an unknown mode, a mode in the wrong case and a spare argument each exit 64 with a
  usage line on stderr and nothing on stdout;
- runs `buggy` and `fixed` 50 times each (`RUNS=n` changes the count) and `wide` 10 times (`WIDE_RUNS=n`) and, for every
  run, checks the exit status, the shape of the stdout line, that the report has one evaluation each of both assert
  lines, the four witness values, no incomplete-catalog notice, and that **the detector and the program agree** (the
  property is false exactly when the final value is not 4,000);
- requires that `fixed` never fires and always ends at 4,000;
- requires that `buggy` fires at least once. If it never fires and the process may use only one CPU (`nproc`, which
  respects affinity and cpusets), the check ends with exit code 77 ("skipped") and `make test` treats that as success,
  with a notice, because the workers cannot overlap there. `LANG_AT_REQUIRE_PARALLEL=1` makes that case a failure
  instead (hosted CI sets it). `RUN_TIMEOUT` (default 60 s) limits each run when `timeout` or `gtimeout` is installed;
- runs the check again on a stand-in whose `fixed` mode really runs `buggy`, and requires it to be rejected, so the
  check itself is known to be able to fail.

The Go fixture also has unit tests of its own (`go test ./...` in `go/`): the busy loop, the order hash, the mode
parser, the line format, the start barrier, and the same buggy-fires/fixed-never-fires check driven through the SDK
without a process. They run with `GOMAXPROCS=2` under `make test`.

## Build record

`scripts/build-record.sh <c|cxx-clang|cxx-gxx|go>` writes a Markdown record of an artifact's build checks: the
toolchain and two builds with equal sha256 (B1), the disassembly of the busy loop and the window (B2), the seccomp
self-test (B3, C and C++), the native runs on at least two CPUs (B4), `strace` of one run to show that every protocol
line is one `write(2)` (B5), the SDK's own tests and conformance goldens (B6) and the source of the modes (B7). It then
captures, from the built artifact, the evidence for the rest of the alignment checklist (`scripts/check-evidence.sh`): a
run with a bad argument and a run with no argument (usage line on stderr, empty stdout, exit 64), the start-barrier
source, the `always` and `sometimes` event lines of one native run with the witness values and one evaluation each, the
property ids, one stdout line, the exit codes 0, 3 and 64, and a grep of the source for reads of the seed, `/proc`, the CPU
count and the clock. The record ends with the checklist: a line is ticked (`[x]`) only when the evidence that proves it
passed, and otherwise left open with the failing evidence named. The record also names its producer (the GitHub run id,
the workflow file and the commit it ran at, the runner image name and version), the commit the artifacts were built from
(`Source commit`, with `+dirty` added when the tree has uncommitted changes) and the sha256 of the artifact, which the job
uploads next to the record, so the bytes in the run are the bytes the record describes. The job then downloads what it
uploaded and runs `scripts/verify-download.sh <artifact-dir> <record>`, a separate step that recomputes the sha256 of the
downloaded bytes (for the Python package, its tree hash) and compares it with `SHA256SUMS` and with the record; run the
same script on an artifact you downloaded yourself. The record is uploaded before that download check runs, so the
downloaded-sha256 evidence is in the job summary of the "Verify the artifact as downloaded" step, not in the record: cite
that step (run and variant) wherever the as-downloaded comparison is asked for. Checks that a not-applicable language skips are shown as `N/A`, never
as `PASS`, and each checklist line names what the tool does not check (`not checked by the tool: ...`). The checks are strict: B2 requires every call to the busy loop to be a load,
the call and a store of the same address (compilers may emit one copy of the window per mode), B5 requires every
protocol line and the result line to be one complete write with a matching return value, and B1 builds twice (the second
time in a different build directory, with a fresh Go build cache) and requires equal sha256.

**Records for the artifacts that are pinned for an acceptance run must come from a native x86-64 Linux host.** B3
(seccomp) and B5 (strace) do not work under an emulated container, and B2 is x86-64 disassembly. The CI workflow has a
manual `fixture-build-records` job (workflow_dispatch, ubuntu-24.04) that runs `build-record.sh` for `c`, `cxx-clang`,
`cxx-gxx` and `go` (and the Python and Rust scripts below) and uploads each record, and the artifact it describes, as
artifacts; any other native x86-64 host works the same way.
`-ffile-prefix-map` (and `-trimpath` in Go) covers the fixture's own sources only: the C SDK library keeps the absolute
directory it was compiled in, so an artifact built on a CI runner and one built in a container differ in sha256. The
pinned artifact therefore comes from one named producer, which the record states (`Producer:`). `scripts/build-record-docker.sh` runs the script in a container with the images pinned by digest (Ubuntu 24.04 for C and
C++, `golang:1.27.3` for Go) and passes the source commit in; on a machine that is not x86-64 it is only a functional
check of the script. Nothing here runs Hermit or needs a server.

### Python's build record

Python has nothing to compile, and the acceptance run pins a package (the fixture, the probe and the `hairspring/` SDK
directory), not one module. `scripts/build-record-python.sh` writes the same kind of record with the checks that make
sense for an interpreter: B1 stages the package twice (`scripts/stage-python.sh`), lists every file with its sha256 and
a tree hash, and requires the two listings to be equal; B2 checks the window on the syntax tree instead of a
disassembly (the disassembly and the seccomp self-test, B3, do not apply); the forbidden-reads check
(`scripts/check-sources-python.sh`) searches `at_py.py` and `probe_py.py` for the seed, `/proc`, the CPU count, the
environment and the clock, and allows `time.monotonic_ns` in the probe alone because its output line carries the
monotonic clock; B4 is 50 native runs per mode, a negative control and the probe line; B5 runs `strace -ff` on one buggy
run and requires every protocol line and the result line to be one whole `write(2)` (`scripts/check-writes.sh`); B6 is the
SDK's tests and conformance; B7 prints the modes. The CI workflow's manual `fixture-build-records` job runs it for
`python` (Python 3.12 from `actions/setup-python`) next to the other variants.

### Rust's build record

`scripts/build-record-rust.sh` records `at-rust` (the static `x86_64-unknown-linux-musl` build of
`sdks/rust/fixtures/lang_at`): B1 checks the toolchain version and that `hairspring-sdk` has no dependencies, then builds
twice into different target directories (the checkout path is mapped away with `--remap-path-prefix`) and requires equal
sha256; B2 disassembles the busy loop and every function that calls it; B3 does not apply (the SDK's own seccomp test runs
in B6); B4 is 50 native runs per mode; B5 runs `strace -ff` on one buggy run; B6 is the SDK's tests in release plus the
conformance tests; B7 prints the constants and the window of each mode. The record covers `at-rust` only.
`scripts/build-record-docker.sh rust` runs it in a container.

`scripts/build-record-probe-rust.sh` records `probe-rust`, the std-only probe program in
[`sdks/rust/fixtures/probe`](../../sdks/rust/fixtures/probe) (no SDK, no dependencies): B1 builds it twice for the static
musl target and requires equal sha256; a source check allows `Instant::now` (the line carries the elapsed monotonic time)
and no other clock, seed, `/proc`, CPU count or environment read; B4 is native runs that check only the format of the
six-field line and the exit status; B5 runs `strace -ff` on one run and requires the line to be one `write(2)`
(`scripts/check-probe-writes.sh`); the probe's own tests run in release. B2, B3, B6 and B7 belong to the AT programs and
are marked not applicable, and the record has no alignment checklist. The CI workflow's manual `fixture-build-records`
job runs it as the `rust-probe` variant; `scripts/build-record-docker.sh rust-probe` runs it in a container.

## The .NET probe

[`dotnet/`](dotnet) holds `LangAtProbe`, a BCL-only program (no SDK, no packages, no property) used to check how .NET
threads are scheduled. It takes no argument. Four `System.Threading.Thread` workers each take a lock once (lock order),
run an unlocked read-spin-write on a plain counter, and queue their number when they finish (arrival order); four tasks
that each await `Task.Delay(1)` queue their number (completion order, the `extra` field). It prints one line, written
with one `write(2)` to stdout, and exits 0:

```
PROBE v1 lang=dotnet lock_order=<16 hex> arrival=<16 hex> rand=<16 hex> vtime_ns=<20 digits> extra=<16 hex>
```

`lock_order`, `arrival` and `extra` are FNV-1a 64 hashes of the worker numbers as bare digits; `rand` is 64 bits from
`Random.Shared`; `vtime_ns` is the difference of two `Stopwatch.GetTimestamp()` readings, the program's only clock read.
Any argument is a usage error (exit 64); an internal failure, such as a worker that did not start or a stdout that cannot
be written, exits 3.

The output is framework-dependent: `LangAtProbe.dll` with its `.runtimeconfig.json` and `.deps.json`, run as
`dotnet LangAtProbe.dll`. The project sets the options that keep the build reproducible (`Deterministic`,
`ContinuousIntegrationBuild`, no debug symbols, no source revision in the assembly, no apphost).
[`dotnet/profiles.manifest`](dotnet/profiles.manifest) lists the environment variables of the two runtime profiles
(arm a: `DOTNET_ROOT` only; arm b: plus `DOTNET_TieredCompilation=0`, `DOTNET_EnableWriteXorExecute=0`,
`DOTNET_gcServer=0` and `DOTNET_PROCESSOR_COUNT=1`); `dotnet/profile-env.sh` prints one arm.

```sh
make -C dotnet test      # the unit checks (selftest/), the build, then test_probe.sh against the built program
make test-dotnet         # the same, from this directory (not part of `make test`: it needs the .NET 10 SDK)
```

`scripts/build-record-probe-dotnet.sh` records `probe-dotnet`: B1 publishes it twice from two copies of the sources in
different directories (with different file times) and requires equal sha256 for each of the three files and checks that
the restore resolved no library; the SDK and runtime versions are checked against the pinned 10.0.402 and 10.0.13; a
source check (`scripts/check-sources-dotnet.sh`) allows `Stopwatch.GetTimestamp` (exactly once,
`scripts/check-stopwatch-use.sh`) and no other clock, seed, `/proc`, CPU count or environment read; B4 is native runs that
check only the format of the line and the exit status, under the base environment and under each profile; B5 runs
`strace -ff` on one run per profile and requires the line to be one `write(2)` (`scripts/check-probe-writes.sh --any-fd`:
the runtime duplicates descriptor 1 for its console stream, so the line appears on another descriptor); the
unit checks run on a clean copy. B2, B3, B6 and B7 are marked not applicable, and the record has no alignment checklist.
The artifact is identified by the tree hash of its three files. The CI workflow's manual `fixture-build-records` job runs
it as the `dotnet-probe` variant (with the pinned SDK from `actions/setup-dotnet`);
`scripts/build-record-docker.sh dotnet-probe` runs it in a container whose image is pinned by digest.

## Compilers

C: gcc and clang. C++: clang++ is the supported compiler; g++ builds and reports too because every Hairspring call site
here is in an ordinary function (`main`), not in a template, lambda or other inline function, which g++ does not place
in the catalog (see [docs/c-sdk.md](../../docs/c-sdk.md)). The C++ program's helper functions contain no call site.

## The Python fixture

`python/at_py.py` is one program with the mode as its first argument: `buggy`, `fixed` (the same three steps under a
`threading.Lock`) or `wide` (`buggy` with a ten times longer busy loop), plus an optional second argument
`--switch=1.0` (the interpreter's thread switch interval, set before any thread starts). Four threads, 1,000
iterations each, a busy loop of 1,000 rounds of pure Python between the read and the write of a shared attribute, a
start barrier. Properties `py.counter_exact` and `py.all_started`; one stdout line,
`LANGAT v1 lang=py mode=<mode> final=<20 digits> expect=<20 digits> order=<16 hex> rand=<16 hex>`; exit 0 whatever the
property says (3 for an internal failure, 64 for a usage error). It finds the SDK next to itself or in
`sdks/python/src`, so it runs under `python -I` too.

```sh
make -C python test                 # unit tests, then the native self-check and its negative control
python3 python/check_native.py --runs 50 --switch-arm   # the numbers: how often each mode loses an update
```

`python/probe_py.py` is a no-SDK companion: four threads, a lock, an unlocked counter and a `queue.Queue`, and one stdout
line of hashes (lock order, finishing order, a random value, elapsed monotonic time, queue drain order).

The self-check has the same rules as the others (the detector and the program agree on every run, the locked twin never
fires, the racy program fires at least once; unlike the others there is no exit 77 "skipped", because the interpreter
lock switches threads on one CPU too), and prints how often each mode
fired and the spread of the final values. [docs/python-sdk.md](../../docs/python-sdk.md) records the numbers; in short,
CPython's interpreter lock hands over every few milliseconds inside the busy loop, so `buggy` loses updates in
essentially every run, and `--switch=1.0` removes the race.

## What this does not do

These are native checks. They show that the race exists and that the detector can see it. They say nothing about how any
scheduler behaves, and the native counts above are not rates for any other setting.
