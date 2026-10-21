# Language fixtures: a planted lost-update race in C, C++ and Go

Three small programs with the same planted race, each with a locked twin, each reporting through its language's
Hairspring SDK ([`sdks/c`](../../sdks/c) for C and C++, [`sdks/go`](../../sdks/go) for Go). They are test subjects for
checking that a language's runtime, its SDK and the `process` backend work together; they are not examples of good
concurrent code.

## The program

Four workers each do 1,000 times:

1. read a shared counter;
2. run a busy loop of 200 rounds of user code (no system call, no sleep);
3. write back the value read plus one.

The read and the write are separate relaxed atomic operations, so the program has no data race in the language's
memory model, but an update can be lost when two workers read the same value. The **locked twin** is the same program
with a mutex held across the three steps (`pthread_mutex_t`, `std::mutex`, `sync.Mutex`); it cannot lose an update.

| Fixed parameter | Value | Where |
|---|---|---|
| workers (T) | 4 | `LANG_AT_THREADS`, `race.Workers` |
| iterations per worker (K) | 1,000 | `LANG_AT_ITERS`, `race.Iterations` |
| busy-loop rounds in the window (W) | 200 | `LANG_AT_SPIN`, `race.Spin` |
| expected final value | 4,000 | `LANG_AT_EXPECTED`, `race.Expected` |

W is the same number in every language, and the loop is the same multiply-add chain; the retired branches per round
differ by compiler and are not equalised.

The busy loop has to survive the optimiser. In C and C++ it is an empty `asm volatile` with a memory clobber in every
round, which also stops the compiler from moving the counter's load and store across the loop. In Go the loop's result
is stored to a per-worker atomic after the window. The native self-check below is what shows the window is real: a
build whose loop had vanished would almost never lose an update.

The memory orders differ between the languages. C and C++ use relaxed loads and stores. Go's atomics are sequentially
consistent, so the Go counter's store is a stronger operation (a locked exchange on x86). The window is the busy loop
in every language and the lost update does not depend on the order, but the programs are not instruction-for-instruction
equivalent and the native rates should not be compared across languages.

## What each program reports

- **Property** `<lang>.counter_exact` (`always`, ids `c.`, `cpp.`, `go.`): the final counter equals 4,000. It is
  evaluated once, after the workers are joined, with the final value and the expected value as its two witness values.
- **Coverage marker** `<lang>.all_started` (`sometimes`): all four workers started.
- **Work**: one operation per iteration (4,000 in total).
- **Stdout**: one fixed-width line, `lang=<id> final=<10 digits> order=<16 hex> rand=<16 hex>`. `order` is FNV-1a over
  the worker ids in the order they finished; `rand` is one value from the system's entropy source, so a run that leaks
  entropy into its output shows it.
- **Exit status**: 0 whatever the property says. The verdict is the protocol report, not the exit code.

## Layout

```
common/lang_at.h       the parts C and C++ share: parameters, busy loop, order hash, stdout line
c/                     src/lost_update.c, Makefile (racy and locked builds, -DLANG_AT_LOCKED for the twin)
cpp/                   src/lost_update.cc, Makefile (clang++ by default; CXX=g++ works)
go/                    main.go, internal/race (the program and its unit tests), Makefile (-tags locked for the twin)
scripts/check-native.sh  the native self-check shared by the three Makefiles; run-checks.sh adds the negative control;
                       test-scripts.sh tests both with a fake program
Makefile               make test: all three
```

## Build and test

```sh
make test                  # everything below
make -C c test CC=clang    # or CC=gcc
make -C cpp test CXX=clang++   # or CXX=g++
make -C go test            # needs Go 1.24 or newer
```

Each language's `make test` builds the racy program and its locked twin, then:

- runs both 50 times natively (`RUNS=n` changes the count) and, for every run, checks the exit status, the shape of
  the stdout line, that the report has both assert lines, and that **the detector and the program agree** (the
  property is false exactly when the final value is not 4,000);
- requires that the locked twin never fires and always ends at 4,000;
- requires that the racy program fires at least once. If it never fires and the process may use only one CPU
  (`nproc`, which respects affinity and cpusets), the check ends with exit code 77 ("skipped") and `make test`
  treats that as success, with a notice, because the workers cannot overlap there. `RUN_TIMEOUT` (default 60 s)
  limits each run when `timeout` or `gtimeout` is installed;
- feeds the racy program in as if it were the locked twin and requires the check to reject it, so the check itself is
  known to be able to fail.

The Go fixture also has unit tests of its own (`go test ./...` in `go/`): the busy loop, the order hash, the line
format, and the same racy-fires/locked-never-fires check driven through the SDK without a process. They run with
`GOMAXPROCS=2` under `make test`.

## Compilers

C: gcc and clang. C++: clang++ is the supported compiler; g++ builds and reports too because every Hairspring call
site here is in an ordinary function (`main`), not in a template, lambda or other inline function, which g++ does not
place in the catalog (see [docs/c-sdk.md](../../docs/c-sdk.md)). The C++ program's helper types have member functions,
but none contains a call site.

## What this does not do

These are native checks. They show that the race exists and that the detector can see it. They say nothing about
how any scheduler behaves, and the native counts above are not rates for any other setting.
