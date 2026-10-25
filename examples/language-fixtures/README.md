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
java/                  the Java probe (no SDK): src/langat, test/, profiles.tsv, manifest.mf, Makefile;
                       the Java fixture (langat.Main, with the Java SDK shaded in): at/src, at/test, at/manifest.mf
dotnet/                the BCL-only .NET probe: src, selftest, test_probe.sh, profiles.manifest, Makefile;
                       the .NET fixture (LangAt.dll, with the .NET SDK beside it as Hairspring.dll): at/src, at/test
tests/seccomp_window.c test-only: the window makes no system call (used by the build record, Linux only)
scripts/check-native.sh  the native self-check of one artifact; run-checks.sh adds the negative control;
                       check-sources.sh, check-disasm.sh; check-java-compiled.sh (PrintCompilation) and
                       run-java-profile-checks.sh (the Java self-check under each profile's flags);
                       test-scripts.sh tests all of them with a fake program
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

`lock_order`, `arrival` and `extra` are FNV-1a 64 hashes of the worker numbers as bare digits; `rand` is
`Random.Shared.NextInt64()` as an unsigned number (a non-negative 63-bit value, so the top bit of the field is 0); `vtime_ns` is the difference of two `Stopwatch.GetTimestamp()` readings, the program's only clock read.
Any argument is a usage error (exit 64); an internal failure, such as a worker that did not start or a stdout that cannot
be written, exits 3.

The output is framework-dependent: `LangAtProbe.dll` with its `.runtimeconfig.json` and `.deps.json`, run as
`dotnet LangAtProbe.dll`. The project sets the options that keep the build reproducible (`Deterministic`,
`ContinuousIntegrationBuild`, no debug symbols, no source revision in the assembly, no apphost).
[`dotnet/profiles.manifest`](dotnet/profiles.manifest) lists the environment variables of the two runtime profiles
(arm a: `DOTNET_ROOT` only; arm b: plus `DOTNET_TieredCompilation=0`, `DOTNET_EnableWriteXorExecute=0`,
`DOTNET_gcServer=0` and `DOTNET_PROCESSOR_COUNT=1`); `dotnet/profile-env.sh` prints one arm.

The runtime needs ICU. Before running the probe on a machine, check (read-only) that libicu is installed, for example
`ls /usr/lib/x86_64-linux-gnu/libicuuc.so.74*`: without ICU the runtime aborts at startup, which would look like a crash of
the probe. `DOTNET_SYSTEM_GLOBALIZATION_INVARIANT` is not set in either profile.

```sh
make -C dotnet test      # the unit checks (selftest/), the build, then test_probe.sh against the built program
make test-dotnet         # the same, from this directory (not part of `make test`: it needs the .NET 10 SDK)
```

`scripts/build-record-probe-dotnet.sh` records `probe-dotnet`: B1 publishes it twice from two copies of the sources in
different directories (with different file times) and requires equal sha256 for each of the three files and checks that
the restore resolved no library; the SDK version is checked against the pinned 10.0.402. The program is then run (B4, B5) on
the pinned runtime 10.0.13: its tarball is checked against the published sha512 (`scripts/extract-runtime.sh`) and
extracted, and that directory is `DOTNET_ROOT` for every run, so the record cannot silently use a newer 10.0.x patch
(`RUNTIME_SOURCE=installed` uses the installed runtime on a machine that is not x86-64 Linux; it still has to be 10.0.13).
A source check (`scripts/check-sources-dotnet.sh`) allows `Stopwatch.GetTimestamp` in `Clock.cs` only (exactly once,
`scripts/check-stopwatch-use.sh`) and no other clock, seed, `/proc`, `/sys`, CPU count or environment read; B4 is native
runs that check only the format of the line and the exit status, under the base environment and under each profile; B5 runs
`strace -ff` on one run per profile and requires the line to be one whole `write(2)` to a duplicate of descriptor 1, no
write at all to descriptors 1 or 2, stdout equal to exactly the line and stderr empty (`scripts/check-probe-writes.sh
--dup-of-stdout`: the runtime duplicates descriptor 1 for its console stream, so the line appears on another descriptor); the
unit checks run on a clean copy. B2, B3, B6 and B7 are marked not applicable, and the record has no alignment checklist.
The artifact is identified by the tree hash of its three files. The CI workflow's manual `fixture-build-records` job runs
it as the `dotnet-probe` variant (with the pinned SDK from `actions/setup-dotnet`);
`scripts/build-record-docker.sh dotnet-probe` runs it in a container whose image is pinned by digest.

## .NET fixture

[`dotnet/at/`](dotnet/at) holds the .NET lost-update fixture: `LangAt.dll`, one program with the three modes (`buggy`, `fixed`,
`wide`) of the contract above, run as `dotnet LangAt.dll <mode>`. It reports through the .NET SDK in
[`sdks/dotnet`](../../sdks/dotnet), which is published beside it as `Hairspring.dll`; the artifact is those two dlls and
`LangAt.runtimeconfig.json` and `LangAt.deps.json`. It has not been run under Hermit, and .NET is not supported on this
build.

Four threads wait at a start barrier, then each does 1,000 times `v = counter; Spin(W); counter = v + 1` on one plain `long`
field (`W` is 1,000, or 10,000 in `wide`; `fixed` holds a `lock` across the three steps). `Spin` is marked `NoInlining`, so the
window holds an opaque call; the SDK's work counter and the busy loop's result (a static field) are written after the store.
The main thread then evaluates `dotnet.counter_exact` (with the witness values `[final, 4000, 4, 1000]`) and
`dotnet.all_started`, writes the one `LANGAT v1 lang=dotnet ...` line with one `write(2)` on descriptor 1, and exits 0; exit 3
means an internal failure and 64 a usage error. `rand` is `Random.Shared.NextInt64()`. The environment profiles are in
[`dotnet/profiles.manifest`](dotnet/profiles.manifest) (the probe's). `scripts/check-dotnet-jit.sh` (`make -C dotnet at-jit`) reads the JIT's own listing of the worker loops
(`DOTNET_JitDisasm`) in the default environment and under both profile arms, lists every compilation with its tier, and
requires each to show the load of the counter, one call of `Spin` and the store of the same field. It covers the compilations of
that run only.

```sh
make -C dotnet at-test      # self-test, source check, native self-checks with a negative control, both profile arms
make -C dotnet at-build     # publish into build/at
make test-dotnet            # the probe and the fixture
```

`scripts/build-record-dotnet.sh` writes the build record: B1 publishes the four files twice from two copies of the sources in
different directories with different file times and requires equal sha256 for each (the fixture maps the repository root to
`/_/` in its compiler path map, because the SDK's call-site parameters write source paths into the assembly); R1 runs the program on the pinned runtime
extracted from a checked tarball; B2 runs that JIT check and B3 is marked not applicable; B4 is 50 native runs per mode with a negative control, in the default environment and under each profile arm
(`scripts/run-dotnet-profile-checks.sh`); B5 runs `strace -ff` on one run without a profile and one per arm and requires every
protocol line (descriptor 2) and the result line (descriptor 1) to be one `write(2)`; B6 runs the SDK's tests and conformance;
B7 prints the modes. The alignment checklist is ticked from the evidence. The `dotnet` variant of the manual
`fixture-build-records` job runs it (with the pinned SDK from `actions/setup-dotnet`, on the runtime extracted from the
tarball).

## Java probe

[`java/`](java) holds a small JDK-only program, `lang-at.jar` (class `langat.Probe`), used to check how Java platform
threads are scheduled. It uses no SDK and has no property or modes, and it takes no argument (any argument prints a usage
line on stderr and exits 64). Four `java.lang.Thread` workers take a `synchronized` monitor, run a loop on a plain
`long` field with no guard, and report their finish on a `LinkedBlockingQueue`; a `ConcurrentHashMap` gives a fourth
structure. It prints exactly one line, with one write to the stdout descriptor, and exits 0:

`PROBE v1 lang=java lock_order=<16 hex> arrival=<16 hex> rand=<16 hex> vtime_ns=<20 digits> extra=<16 hex>`

`lock_order` and `arrival` are FNV-1a 64 hashes of the worker numbers written as bare digits, `extra` hashes the map's
keys in iteration order, `rand` is the first value of a new `java.util.Random` (which JDK 21 seeds from a counter and
`System.nanoTime()`), and `vtime_ns` is the monotonic clock difference between the start and the end of the run. Exit 3
means an internal failure (a thread did not start or died, or the line could not be written).

The jar names `langat.Probe` as its `Main-Class` because it holds the probe alone; a jar that also holds another program
must not.

`java/profiles.tsv` lists the JVM flag profiles the program is run with (two profiles, arm `a` and arm `b`); the flags are
not part of the jar. The jar is built with `javac --release 21 -Xlint:all -Werror` and written with a fixed entry timestamp
and its entries in sorted order, so two builds with the same JDK give the same bytes.

```sh
make -C java test      # self-test of the classes, then native tests of the jar (line format, usage errors, exit 3)
make -C java flags     # every profile of profiles.tsv: valid JVM options, one clean line, no JVM warning
make test-java         # both, then the Java fixture's own tests (see "Java fixture")
```

`scripts/build-record-probe-java.sh` writes the build record: B1 builds the jar twice (two directories, two time zones)
and requires equal sha256, and checks the sorted entries and the single timestamp; a source check
(`scripts/check-sources-java.sh`) allows two `System.nanoTime` reads and no other clock, environment, `/proc`, CPU count or
process-table read; the flag profiles are checked; B4 is native runs that check only the format of the line and the exit
status; B5 runs `strace -ff` on one run per profile and requires the line to be one `write(2)`
(`scripts/check-probe-writes.sh`). The record has no alignment checklist. The CI workflow's manual
`fixture-build-records` job runs it as the `java-probe` variant (Temurin 21 from `actions/setup-java`; the record fails if
the JDK is not the pinned build); `scripts/build-record-docker.sh java-probe` runs it in a container with the pinned
Temurin tarball. Under an emulated container `strace` sees nothing, so B5 needs a native host.

## Java fixture

`java/at/` holds the Java lost-update fixture: class `langat.Main`, one program with the three modes (`buggy`, `fixed`,
`wide`) of the contract above, built with the Java SDK in [`sdks/java`](../../sdks/java). The SDK, the probe's classes
and the fixture's go through one `javac --release 21 -Xlint:all -Werror` run into one reproducible `lang-at.jar` (a fixed
entry timestamp, sorted entries, no module descriptor, `Main-Class: langat.Main`); the probe-only jar above is a different
artifact. The mode is `argv[1]` and the JVM flags are not part of the jar: `java/profiles.tsv` lists the two profiles (arm
`a`: interpreter only, arm `b`: C1 only) the program is run with.

Four platform threads wait at a start barrier, then each does 1,000 times `v = counter; spin(W); counter = v + 1` on one
plain `long` field (`W` is 1,000, or 10,000 in `wide`; `fixed` holds a `synchronized` monitor across the three steps). The
SDK's work counter and the busy loop's result (a volatile field, so the loop cannot be dropped) are written after the
store. The main thread then evaluates `java.counter_exact` (with the witness values `[final, 4000, 4, 1000]`) and
`java.all_started`, writes the one `LANGAT v1 lang=java ...` line with one call on `FileOutputStream(FileDescriptor.out)`,
and exits 0; exit 3 means an internal failure (a thread did not start, the barrier broke, the line could not be written) and
64 a usage error. `rand` is the first value of a new `java.util.Random`. The window survives the JIT because
HotSpot only moves a plain load past a loop in code it compiles, and it compiles a method (or a loop in it) when the method's
invocation and back-edge counters, summed over all threads, pass a threshold: each worker method is called 4 times and its
loop passes 4 x 1,000 = 4,000 times in all, below the thresholds. The record checks it at run time, with
`-XX:+PrintCompilation` (see [docs/java-sdk.md](../../docs/java-sdk.md)).

```sh
make -C java at-test        # self-test, source check, native self-checks with a negative control, both flag profiles
make -C java at-jar         # build/at/lang-at.jar
make -C java at-arms        # each profile: valid flags, fixed exact, how often buggy fires
make test-java              # the probe and the fixture
```

`scripts/build-record-java.sh` writes the build record: B1 builds the jar twice (two directories, two time zones) and
requires equal sha256, and checks the layout; B2 reads the window off the bytecode (`scripts/check-window-java.sh`: a load of
the counter, one call of `spin`, a store, nothing else that calls, locks or allocates between them); B3 is a code-review
statement (the JVM makes system calls on other threads, so there is no seccomp test; the statement says plainly that `spin`
is called inside the window, that a compile wait under arm b and a safepoint stop under both arms can happen there, and that
this only makes the race easier to reach, and leaves the signature to the reviewer); F3 runs every mode under
`-XX:+PrintCompilation` with the default flags and each profile (`scripts/check-java-compiled.sh`) and requires that neither
worker loop is ever compiled; F4 requires the probe's classes in `lang-at.jar` to be byte-identical (sha256, one by one) to
those of the probe-only jar; the source check allows no clock, environment, `/proc`, CPU count or process-table read in the fixture; the flag
profiles are run (`scripts/check-java-arms.sh`); B4 is 50 native runs of `buggy` and `fixed`, `wide` as a contract check
and a negative control, with the default flags and again under each profile's flags (`scripts/run-java-profile-checks.sh`); B5 runs `strace -ff` on one run without flags and one per profile and requires every protocol line
and the result line to be one `write(2)`; B6 runs the SDK's tests and conformance; B7 prints the modes. The alignment
checklist is ticked from the evidence. The `java` variant of the manual `fixture-build-records` job runs it (Temurin 21 from
`actions/setup-java`; the record fails if the JDK is not the pinned build); `scripts/build-record-docker.sh java` runs it in a
container with the pinned Temurin tarball.

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
