# Python SDK

The Python SDK (`sdks/python`, package `hairspring`) lets a Python program declare assertions that Hairspring tracks
across trials. It speaks [guest protocol v1](guest-protocol.md), writes the same lines as the [C SDK](c-sdk.md)
(checked byte for byte, see "Conformance"), is pure Python with no dependencies, supports CPython 3.12 or newer on Linux
and macOS, and is not published to a registry.

A call site only updates counters in memory; the program reports once, when the interpreter exits.

```python
import hairspring as hs

hs.always(balance >= 0, "bank.balance_non_negative", "no overdraft", balance)
hs.sometimes(waited, "bank.writer.waited", "a writer waited for the lock")
hs.reachable("bank.recovery.ran", "recovery ran")
hs.unreachable("bank.bad_state", "never expected")
hs.work(1)                       # operations done, reported as work.ops
hs.thread("writer", 0)           # stable thread key "writer-0"
```

Use it from the source tree with `PYTHONPATH=sdks/python/src`, or install it from `sdks/python` (`pip install ./sdks/python`).
A runnable example is `sdks/python/examples/quickstart.py`.

> **A site that never runs is invisible, not unmet.** Python cannot see a call site before it executes, so a
> `reachable`, `sometimes` or `always` in code the run never reaches is simply absent from the report, and the run
> cannot flag it as UNMET. That is the main thing those kinds exist to find. **Call `hs.declare(hs.Kind.REACHABLE, id,
> msg)` once, at import time, for every site whose absence matters.**

> **Evaluations made in a forked child are lost.** `os.fork()` and `multiprocessing` with the `fork` start method (the
> default on Linux for Python 3.12 and 3.13) give the child a copy of the SDK's state, and only the process that started
> the SDK reports. Everything a forked child evaluates is silently dropped: it is not reported and not counted in
> `end.dropped`. Use `multiprocessing.get_context("spawn")` (or `"forkserver"`) for workers that assert; a spawned child
> is a fresh interpreter that reports under its own tag `<parent>.s<pid>`. (A child marker that counted forked-child
> evaluations would need the child to tell the parent, which the report path has no channel for, so this is documented
> rather than half-fixed.)

## API

| Function | Meaning | Trial-level failure | Run-level UNMET |
|---|---|---|---|
| `always(cond, id, msg="", *vals)` | must hold at every evaluation | any false | never evaluated |
| `always_or_unreachable(cond, id, msg="", *vals)` | same, but never reaching it is fine | any false | none |
| `sometimes(cond, id, msg="", *vals)` | must hold at least once per run | none | never true |
| `reachable(id, msg="", *vals)` | must be reached at least once per run | none | never hit |
| `unreachable(id, msg="", *vals)` | must never be reached | hit | none |

| Function | Meaning |
|---|---|
| `declare(kind, id, msg="")` | puts a site in the catalog without evaluating it. **Needed for any site that might never run**, or it is invisible rather than unmet (see "The catalog is built as the program runs") |
| `work(n)` | adds `n` to the process's operation count, reported as `work.ops` |
| `thread(label, index)` | gives the calling thread the key `label-index`. Returns `False` if another thread has the key or the thread shares the overflow slot, and the thread then keeps its claim-order key `t#<n>`, flagged `key_stable:false`. Raises `ValueError` for an empty label or a key over 64 bytes. Call it before the thread does anything else |
| `report()` | writes the report now (reason `explicit`); the first report wins, and evaluations after it are not reported |
| `exit(status=0)` | records the status and calls `sys.exit(status)`, so the exit report carries it. Call it from the main thread (in another thread `sys.exit` only ends that thread) |

* `cond` is judged by its truth value. `vals` is 0 to 4 integers (anything with `__index__`, so `bool` counts), converted
  to signed 64 bits like a C cast, captured at the first true and the first false evaluation (`vals` in the protocol;
  missing values are 0). A fifth value, or a value that is not an integer, raises `TypeError` and counts nothing.
* `id` is 1 to 128 bytes of UTF-8; bytes outside `[A-Za-z0-9._:/-]` are sent as `_` (as in the C SDK), so ids that
  differ only in such bytes are one id. `msg` is at most 512 bytes; bytes that are not printable ASCII, and `"` and `\`
  are handled as in the C SDK (`?` and an escape). An empty or over-long id, an id containing a NUL character (the
  wire form would stop there), or an over-long message, raises
  `ValueError` at the first evaluation. (The C SDK refuses these at compile time, the Rust SDK too; Python has no
  compile time, so the first run shows the mistake. Nothing is recorded for the call.)
* The same id and kind from several places is one site: the report merges them into one `assert` line, and `end.sites`
  counts distinct ids. An id used with two kinds is a catalog conflict the engine reports; the `assert` line carries the
  first kind's counts.
* The location in the `catalog` line is the first call's file and line in the caller (or `declare`'s).
* `end.status` is advisory (the engine uses the operating system's wait status). Python's exit handlers cannot see the
  status of a plain `return`, `sys.exit(n)` or an uncaught exception, so `end.status` is 0 unless `exit(n)` recorded one.

## Environment

| Variable | Effect |
|---|---|
| `HAIRSPRING_EVENTS_FILE=path` | write events to this file (`O_APPEND`, `O_NOFOLLOW` so a symlink is refused, `O_NONBLOCK`, and it must be a regular file) instead of stderr; if it cannot be opened or is not a regular file, events go to stderr and `end.dropped` counts it. Ignored in a set-uid or set-gid process (`AT_SECURE` from `/proc/self/auxv` on Linux, where an unreadable vector counts as privileged; real and effective ids elsewhere) |
| `HAIRSPRING_EVENTS=off` | record as usual but write nothing |
| `HAIRSPRING_PROC=tag` | the root process tag: 1 to 64 bytes of `[A-Za-z0-9._:/-]`, not `_eval`; default `p1`. An invalid tag is refused (the default is used and `end.dropped` is incremented), never shortened |
| `HAIRSPRING_PROC_PID` | set by the SDK; do not set it |
| `HAIRSPRING_SIGNAL_REPORT` | **not supported**: ignored, and `hello.signal_handlers` is always false |

At import time the SDK duplicates descriptor 2 (or opens the events file) to a number of 200 or more
(`fcntl(F_DUPFD_CLOEXEC)`, 20 if the descriptor limit is too low) and writes only to that copy, so a program that closes
stderr and then opens a data file cannot receive the report. It remembers which file the copy refers to and checks it
with one `fstat` before the report. As in the C SDK this is best effort: programs under test must not `dup2` onto or
close that descriptor. The [C SDK page](c-sdk.md#descriptors) has the longer discussion.

## What is written, and when

The lines and their widths are those of the C SDK, so see [c-sdk.md](c-sdk.md#what-is-written-and-when) for the full
account. Every line is `HAIRSPRING_EVENT ` plus one JSON object, at most 4096 bytes, written with one `write(2)` in the
normal case. Counters are right-aligned in 20 characters, witnesses are padded to 197 characters and thread keys to 64,
so a line's length does not depend on any counter or on whether a failure happened.

**Everything is written at the end**: `hello`, then one `catalog` line per distinct (id, kind) pair, one `assert` per
id, `work`, one `thread` line per thread (up to 256), then `end`. At most once per process, from `atexit` or
`report()`, and only from the process that started the SDK. This differs from the C and Rust SDKs, which send `hello`
and the catalog at startup: Python cannot know its call sites before they run, and `hello` carries the catalog's hash.
The consequence is that **a process killed before it reports (a timeout, `SIGKILL`, `os._exit`, a fatal signal) leaves
no lines at all**, where a C program that was killed leaves its catalog. The engine treats a process without an `end`
as `report_missing`; with no line at all it has nothing to say about the process.

### The catalog is built as the program runs

A site is catalogued the first time it is evaluated. A `reachable` or `sometimes` in code that never runs is therefore
invisible, and a run cannot report it as unmet, which is the main thing those kinds are for. **Declare such sites once,
at import time:**

```python
hs.declare(hs.Kind.REACHABLE, "wal.reset.reached", "the WAL was reset")
```

Evaluating the same id and kind later (`hs.reachable("wal.reset.reached")`) is the same site. A declared site that is
never evaluated is reported with zero hits, as the C SDK reports a call site that never ran. Declarations and
evaluations past 16,384 distinct ids are not recorded and are counted in `end.dropped`, one per evaluation.

### Threads

A fixed table of 256 slots, claimed under the SDK's lock on a thread's first SDK call. Slots are never recycled and
keys cannot be reused: the table is for fixed pools. Past the 255th thread the rest share the last slot (`t#overflow`,
`key_stable:false`) and each is counted in `end.dropped`. Ordinals in witnesses are per thread, as in the C SDK.

### Processes

The tag travels in the environment (`HAIRSPRING_PROC`, `HAIRSPRING_PROC_PID`), exactly as in the C SDK.

| What the program does | What the report shows |
|---|---|
| `subprocess`, `multiprocessing` with `spawn` or `forkserver` | the child inherits the parent's tag and a pid that is not its own, so it takes `<parent>.s<pid>` and says `tag_stable:false` in its `hello`. It reports at its own exit |
| `os.exec*` before the parent has reported | the image keeps the tag; the parent never reports, so the tag appears once |
| `os.exec*` after `report()` | the image takes `<tag>.x1` |
| `os.fork`, `multiprocessing` with `fork` | **there is no fork handler** (`<parent>.f<n>` tags). The child shares the parent's state and reports nothing of its own: the SDK records the pid of the process that started it and writes the report only from that process, so a child that calls `sys.exit` does not repeat its parent's report. Anything a forked child evaluates is lost. A `multiprocessing` fork child ends with `os._exit` and runs no exit handler anyway |

The SDK takes its lock across `fork` (`os.register_at_fork`), so a child never inherits it held by a thread that no
longer exists.

## Thread safety

Every change to a counter happens with one reentrant lock held. The GIL does not make `count += 1` safe: CPython can
switch threads between the load and the store, so without the lock evaluations from two threads are lost. (CPython 3.12
happens to switch threads only at a few points, so a short `count += 1` is rarely interrupted there; that is an
implementation detail, not a guarantee, and it does not hold for other versions or a free-threaded build.) The tests
run 8 threads against one site with `sys.setswitchinterval(1e-6)` and check the totals exactly, and a second test
installs a tracer that yields the interpreter lock after every opcode of the SDK's own code, which makes every
unprotected read-modify-write lose updates: it fails when the lock is taken out.

* A report is written from a **snapshot** taken under the lock, so every count, witness and thread total belongs to one
  moment. A witness is only ever written behind a non-zero count (the parser rejects a line that pairs them). With one
  kind per id, the evaluations the thread lines add up to equal the hits the `assert` lines add up to in every
  snapshot, because both are changed inside one critical section. That equality is **not** promised in two cases: an id
  used with two kinds (the `assert` line carries only the first kind's counts, so the other kind's evaluations are in
  the thread totals but not in any `assert` line), and an asynchronous exception (`KeyboardInterrupt`, or one raised in
  a thread with `PyThreadState_SetAsyncExc`) landing between the two updates. The tests take snapshots while four
  threads evaluate and check the single-kind case.
* The lock is reentrant because a signal handler, or a finaliser run by the garbage collector, can call the SDK on the
  thread that is inside the SDK's own critical section. With a plain lock that would hang the thread. With this one it
  costs, at worst, a witness taken from a half-updated site; counts stay exact for every finished call.
* The critical sections are a few attribute updates. The report's lines are written outside the lock.

## Call-site cost

A call does this: check the number and type of `vals`; find the site in a dict keyed by `(id, kind)`; find the thread's
slot in a `threading.local`; take the lock; bump the thread's evaluation count and the site's `trues` or `falses`; on
the first evaluation of a side, store the witness. No system call, no I/O and no allocation beyond a small tuple for
the witness. **It is not constant time with respect to the condition**: Python offers no way to make a true and a false
evaluation execute the same instructions, and the C SDK's argument about schedule perturbation does not carry over. The
SDK's own cost is small next to the interpreter's, but it does take a lock, and a lock acquire is a point where the GIL
can be handed over; programs under test that rely on the *absence* of switches inside a window must keep SDK calls out
of it (the fixture does).

Measured once on macOS arm64, CPython 3.12.2, 1,000,000 calls through `timeit` with events off: `always` with one value
about 0.66 microseconds, `always` with none about 0.44, `reachable` about 0.42, `work` about 0.36 (an empty lambda
about 0.02). These are one machine's figures and say nothing about other hosts.

## The planted race

`examples/language-fixtures/python/at_py.py` is one program with three modes chosen by its first argument:

* `buggy`: 4 threads, each doing 1,000 iterations of: read a shared integer attribute, run a busy loop of 1,000 rounds of
  pure Python (a multiply-add chain in the program's own `spin`), write back the value read plus one. The read and the
  write are separate operations, so an update is lost whenever the interpreter hands the lock to another thread inside
  the loop. There is no system call, lock or SDK call between the read and the write. It does allocate: big-integer
arithmetic, a `range` and a call frame in every iteration. CPython offers no way around that for pure-Python code, so
unlike the C fixture's window (which a seccomp test proves syscall-free) this window is "no system call and no SDK
call", not "no allocation". The busy loop's result goes to `sink.value` after the store.
* `fixed`: the same three steps under a `threading.Lock`.
* `wide`: `buggy` with a busy loop ten times longer.

Properties: `py.counter_exact` (`always`, the final counter equals 4,000, witness values `[final, 4000, 4, 1000]`) and
`py.all_started` (`sometimes`), each evaluated once in the main thread after the workers are joined. Output: one line on
stdout, `LANGAT v1 lang=py mode=<mode> final=<20 digits> expect=<20 digits> order=<16 hex> rand=<16 hex>`, and exit 0
whatever the property says (3 for an internal failure, 64 for a usage error). The workers wait at a start barrier.
`--switch=1.0` as the second argument sets the thread switch interval to one second before any thread starts. A failed
stdout write exits 3 without a traceback and the report's `end.status` says 3, as in the C and Rust fixtures. The start
barrier has no timeout: a worker that fails or cannot start aborts it, which wakes the others.

The acceptance run starts the fixture as `python3 -I -B` with `PYTHONHASHSEED=0` in the environment. Under `-I`
(isolated mode) `PYTHONHASHSEED` is ignored, so the variable has no effect there: string hash randomization stays on
and its seed comes from the operating system (`getrandom`). That is harmless here, since nothing in the fixture or the
SDK depends on hash order, but it is a source of per-run entropy the setting does not remove.

The native self-check has no "skipped" result (exit 77) as the C, C++ and Go checks do: CPython switches threads on one
CPU too, so the race needs no second CPU, and a racy program that never fires fails the check wherever it runs.

**CPython's lock makes the race common, not rare.** The interpreter's default switch interval is 5 ms and a busy
loop of pure Python is interruptible, so a thread is almost always preempted inside the window. Measured natively
(`check_native.py`, 50 runs of each):

| | macOS arm64, CPython 3.12.2 | Linux aarch64, CPython 3.12.3 (container) | Linux x86-64, CPython 3.12.15 (hosted CI runner) |
|---|---|---|---|
| `buggy` lost an update | 50 of 50; final 1,000 to 1,346 (median 1,124) | 50 of 50; final 1,000 to 2,085 (median 1,184) | 50 of 50; final 1,000 to 2,092 (median 1,106) |
| `wide` lost an update | 50 of 50; final 990 to 1,069 | 50 of 50; final 1,000 to 1,177 | 50 of 50; final 847 to 1,757 |
| `fixed` lost an update | 0 of 50; always 4,000 | 0 of 50; always 4,000 | 0 of 50; always 4,000 |
| `buggy --switch=1.0` lost an update | **0 of 50**; always 4,000 | **0 of 50**; always 4,000 | **0 of 50**; always 4,000 |

With one-second switch slices each thread finishes before it is preempted, so that flag removes the race on this
fixture: it is a flag to study determinism with, not one to hunt with. These native numbers say the race exists and the
detector sees it; they are not rates under any scheduler, and a race that fires natively in every run may still fire
rarely, or never, under a deterministic scheduler that changes when the interpreter's time-driven switch happens.

`examples/language-fixtures/python/probe_py.py` is a small companion with no SDK and no property: four threads, a lock, an
unlocked counter and a `queue.Queue`, one stdout line (`PROBE v1 lang=py lock_order=... arrival=... rand=... vtime_ns=...
extra=...`) of hashes of the lock order, the finishing order, a random value, the elapsed monotonic time and the queue's
drain order. It exists to ask whether those are a function of the scheduler's seed; it accepts the same `--switch=1.0`
argument.

A determinism check alone does not show that the interpreter switched threads at all (under a virtual clock the 5 ms
switch interval may never elapse, and then every run is trivially identical). So the line ends with four more fields:
`racy_final` and `racy_expect` (the unlocked counter after 4 threads x 200 read-spin-write rounds, and 800: their
difference is the number of updates lost, and 0 means no switch ever landed inside a window) and `t_start_ns` and
`t_end_ns` (the monotonic clock before the threads start and after they are joined; `vtime_ns` is their difference).
The original six fields are unchanged. Natively the counter loses updates in essentially every run, and with
`--switch=1.0` it loses none.

## Conformance

* `cargo test -p hairspring-props --test python_sdk` runs the sample programs and the fixture and folds what they write
  with the reference parser. It mirrors the C SDK's `c_sdk` tests (complete undegraded report, a false `always` is a
  violation with its witness, an unmet `reachable`, merged sites, 100 and 300 threads, exit status, two processes on one
  output, a spawned child's tag and `tag_stable`, a catalog conflict, an empty catalog, limits), adds the events file,
  `HAIRSPRING_EVENTS=off`, a closed reader (`SIGPIPE`), a `multiprocessing` child and a forked child, and the planted race.
  `HS_PYTHON` names the interpreter; without it the first of `python3.14`, `python3.13`, `python3.12`, `python3` and
  `python` that is Python 3.12 or newer is used. With none, the tests print a skip notice and pass, unless
  `HS_REQUIRE_PYTHON` is set, which makes a missing or too old Python an error (CI sets it).
* `the_report_matches_the_c_sdks_byte_for_byte` runs the C SDK's `basic` scenario and the Python `basic` sample, which has
  the same ids, messages, values and order (the three sites the C program never reaches are `declare`d). The `assert`,
  `work`, `thread` and `end` lines are equal character for character (including every padding space); `hello` and
  `catalog` are equal once the SDK name and version and the source locations are removed; and the `catalog_hash` is
  equal, which also checks the id hash. This needs `make` and a C compiler, as the C SDK's own conformance tests do.

## Tests

`python3 -m unittest discover -s sdks/python/tests -t sdks/python` (standard library only):

* unit tests of the formatter, every line kind, the catalog hash (including the value the C SDK prints), the thread
  slots (300 claims, key rules), the site table and argument checks, the process identity rules, the `AT_SECURE`
  decision, and the output channel (events file, symlink, FIFO with and without a reader, device, privileged process,
  descriptor taken away, full non-blocking pipe, `SIGPIPE` blocked and consumed);
* `test_runtime.py`: report shape and order, witnesses, merging, conflicts, declared sites, validation, exactness under
  8 contending threads with a one-microsecond switch interval and under a tracer that switches after every opcode,
  snapshot consistency while threads run, 32 simultaneous first claims, reentrancy;
* `test_process.py`: the sample programs as real processes (exit statuses, an uncaught exception, 100 and 300 threads, a
  fork, a fork while another thread is inside the SDK (the child must be able to use it; this fails without the
  `fork` hook), a spawn, `multiprocessing` with `spawn` and with `fork`, `exec` before and after the report, two
  processes on one pipe, a signal handler that calls the SDK, a closed stderr pipe, a program that closes stderr and
  reuses it).

The fixture has its own tests (`examples/language-fixtures/python`, `make test` there): its command line, its stdout
line, its report, and an `ast` check that the window is exactly a read, the busy loop and a write.

Where it has been run natively (for Hermit see the [support matrix](support-matrix.md)): macOS arm64 with CPython 3.12.2 and 3.14 (the SDK tests, the fixture tests
and self-check, the conformance test); Linux aarch64 natively in an Ubuntu 24.04 container with CPython 3.12.3 (the SDK
tests, including the Linux-only pipe and `AT_SECURE` paths, the fixture tests and the 50-run self-check); and Linux
x86-64 in CI with CPython 3.12 (the SDK tests, the conformance test with `HS_REQUIRE_PYTHON`, the fixture tests and the
self-check).

## Differences from the C SDK

* Hello and the catalog are written at the end, with the report (see above).
* Ids and messages are checked at the first evaluation (`ValueError`) instead of at compile time.
* A site that never runs is invisible unless it is `declare`d.
* No `HAIRSPRING_SIGNAL_REPORT` (a fatal signal without a report shows as `report_missing`), no `HS_EVAL_*` per-evaluation
  lines, no fork handler, no `hs_choice` or tape (neither exists in the C SDK yet either), no call-site audit (there is
  no linker section to fall out of: every evaluation registers its own site).
* Linux and macOS only. Like the C SDK, the SDK blocks `SIGPIPE` around the report and consumes a `SIGPIPE` that its own
  write raised, so a reader that has gone away costs the report, not the process (Python normally ignores `SIGPIPE`;
  the SDK does not depend on that).
* Writing the events file: it is opened non-blocking and must be a regular file, as in the Rust SDK. A write to a full
  non-blocking pipe waits for the reader for up to about ten seconds before giving the line up.
* One copy of the SDK per process: importing it under two names (two `sys.path` entries that both hold it) would start
  two runtimes, which report under the same tag.

## Not done

* Signal-time reports, `HS_EVAL_*` equivalents, a fork handler, the tape API, asyncio-aware thread keys (tasks are not
  threads; every task of one event loop shares its thread's slot), free-threaded builds (the lock would make it correct;
  nothing has been run on one).
* Windows.
* Hermit: not supported on this build; the probe program did not complete under Hermit. See the [support matrix](support-matrix.md).
