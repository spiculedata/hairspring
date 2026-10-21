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
| `python/at_py.py` (no build; the file is the artifact) | `make -C python` | `buggy`, `fixed`, `wide`, and `--switch=1.0` as an optional second argument |

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
line is one `write(2)` (B5), the SDK's own tests and conformance goldens (B6) and the source of the modes (B7). It ends
with the alignment checklist to tick by hand. The checks are strict: B2 requires every call to the busy loop to be a load,
the call and a store of the same address (compilers may emit one copy of the window per mode), B5 requires every
protocol line and the result line to be one complete write with a matching return value, and B1 builds twice (the second
time in a different build directory, with a fresh Go build cache) and requires equal sha256.

**Records for the artifacts that are pinned for an acceptance run must come from a native x86-64 Linux host.** B3
(seccomp) and B5 (strace) do not work under an emulated container, and B2 is x86-64 disassembly. The CI workflow has a
manual `fixture-build-records` job (workflow_dispatch, ubuntu-24.04) that runs `build-record.sh` for `c`, `cxx-clang`,
`cxx-gxx` and `go` and uploads each record as an artifact; any other native x86-64 host works the same way.
`-ffile-prefix-map` (and `-trimpath` in Go) covers the fixture's own sources only: the C SDK library keeps the absolute
directory it was compiled in, so an artifact built on a CI runner and one built in a container differ in sha256. The
pinned artifact therefore comes from one named producer, which the record states (`Producer:`). `scripts/build-record-docker.sh` runs the script in a container with the images pinned by digest (Ubuntu 24.04 for C and
C++, `golang:1.27.3` for Go) and passes the source commit in; on a machine that is not x86-64 it is only a functional
check of the script. Nothing here runs Hermit or needs a server.

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

The self-check has the same rules as the others (the detector and the program agree on every run, the locked twin never
fires, the racy program fires at least once, exit 77 "skipped" on a one-CPU process), and prints how often each mode
fired and the spread of the final values. [docs/python-sdk.md](../../docs/python-sdk.md) records the numbers; in short,
CPython's interpreter lock hands over every few milliseconds inside the busy loop, so `buggy` loses updates in
essentially every run, and `--switch=1.0` removes the race.

## What this does not do

These are native checks. They show that the race exists and that the detector can see it. They say nothing about how any
scheduler behaves, and the native counts above are not rates for any other setting.
