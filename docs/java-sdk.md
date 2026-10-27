# Java SDK

The Java SDK (`sdks/java`, package `hairspring`) lets a Java program declare assertions that Hairspring tracks across
trials. It speaks [guest protocol v1](guest-protocol.md), writes the same lines as the [C SDK](c-sdk.md) (checked byte for
byte, see "Conformance"), is pure Java 21 with no dependencies (JDK classes only, no agent, no JNI), supports Linux and
macOS, and is not published to a registry.

A call site only updates counters in memory; the program reports once, when the JVM shuts down.

```java
import static hairspring.Hairspring.*;

always(balance >= 0, "bank.balance_non_negative", "no overdraft", balance);
sometimes(waited, "bank.writer.waited", "a writer waited for the lock");
reachable("bank.recovery.ran", "recovery ran");
unreachable("bank.bad_state", "never expected");
work(1);                     // operations done, reported as work.ops
thread("writer", 0);         // stable thread key "writer-0"
```

Build it with `make -C sdks/java jar` (`build/hairspring-sdk.jar`, module `hairspring`) or put `sdks/java/src` on the class
path. A runnable program is `sdks/java/samples/hssample/Sample.java`.

> **A site that never runs is invisible, not unmet.** Java cannot see a call site before it executes, so a `reachable`,
> `sometimes` or `always` in code the run never reaches is simply absent from the report, and the run cannot flag it as
> UNMET. That is the main thing those kinds exist to find. **Call `declare(Kind.REACHABLE, id, msg)` once, early, for
> every site whose absence matters.**

> **The SDK starts when its class is first used.** Any call starts it, or call `Hairspring.start()`. A program that never
> touches the class reports nothing, not even a `hello`.

## API

All methods are static on `hairspring.Hairspring`. `vals` is `long...`.

| Method | Meaning | Trial-level failure | Run-level UNMET |
|---|---|---|---|
| `always(cond, id, msg, vals...)` | must hold at every evaluation | any false | never evaluated |
| `alwaysOrUnreachable(cond, id, msg, vals...)` | same, but never reaching it is fine | any false | none |
| `sometimes(cond, id, msg, vals...)` | must hold at least once per run | none | never true |
| `reachable(id, msg, vals...)` | must be reached at least once per run | none | never hit |
| `unreachable(id, msg, vals...)` | must never be reached | hit | none |

Each also has a form without `msg` and `vals`.

| Method | Meaning |
|---|---|
| `declare(kind, id, msg)` | puts a site in the catalog without evaluating it. **Needed for any site that might never run** |
| `work(n)` | adds `n` to the calling thread's operation count, reported as `work.ops` (a total over all threads) |
| `thread(label, index)` | gives the calling thread the key `label-index`. Returns `false` if another thread has the key or the thread shares the overflow slot, and the thread then keeps its claim-order key `t#<n>`, flagged `key_stable:false`. Throws `IllegalArgumentException` for an empty label, an index outside `0..4294967295` or a key over 64 bytes. Call it before the thread does anything else |
| `report()` | writes the report now (reason `explicit`); the first report wins, and evaluations after it are not reported |
| `exit(status)` | records the status and calls `System.exit(status)`, so the shutdown hook's report carries it. Do not call it from a shutdown hook |
| `prepare(processBuilder)` | puts the process tag in the environment of a child you are about to start (see "Processes") |
| `start()` | starts the SDK without evaluating anything |

* `cond` is a `boolean`. `vals` is 0 to 4 values, captured at the first true and the first false evaluation (`vals` in the
  protocol; missing values are 0). A fifth value throws `IllegalArgumentException` and counts nothing.
* `id` is 1 to 128 bytes of UTF-8; bytes outside `[A-Za-z0-9._:/-]` are sent as `_` (as in the C SDK), so ids that differ
  only in such bytes are one id. `msg` is at most 512 bytes; bytes that are not printable ASCII, and `"` and `\` are handled
  as in the C SDK (`?` and an escape). An empty or over-long id, an id containing a NUL character, or an over-long message
  throws `IllegalArgumentException` at the first evaluation (the C SDK refuses these at compile time; Java has no
  compile time for them). A `null` id or message throws `NullPointerException`. Nothing is recorded for the call.
* The same id and kind from several places is one site: the report merges them into one `assert` line, and `end.sites`
  counts distinct ids. An id used with two kinds is a catalog conflict the engine reports; the `assert` line carries the
  first kind's counts.
* The location in the `catalog` line is the first call's file (as `package/File.java`) and line, taken from the stack once
  per site, never on the hot path.
* `end.status` is advisory (the engine uses the operating system's wait status). The JVM's shutdown hooks cannot see the
  status of a plain `System.exit(n)`, a normal return or an uncaught exception, so `end.status` is 0 unless `exit(n)`
  recorded one.

## Environment

| Variable | Effect |
|---|---|
| `HAIRSPRING_EVENTS_FILE=path` | write events to this file (appended; a symbolic link in the last component is refused; it must be a regular file) instead of standard error; if it cannot be used, events go to standard error and `end.dropped` counts it. Ignored in a set-uid or set-gid process (`AT_SECURE` from `/proc/self/auxv` on Linux, where an unreadable vector counts as privileged; on other systems the JVM offers no effective id, so the setting is honoured) |
| `HAIRSPRING_EVENTS=off` | record as usual but write nothing |
| `HAIRSPRING_PROC=tag` | the root process tag: 1 to 64 bytes of `[A-Za-z0-9._:/-]`, not `_eval`; default `p1`. An invalid tag is refused (the default is used and `end.dropped` is incremented), never shortened |
| `HAIRSPRING_PROC_PID` | set for children by `prepare`; do not set it |
| `HAIRSPRING_SIGNAL_REPORT` | **not supported**: ignored, and `hello.signal_handlers` is always false |

The report goes to standard error through descriptor 2, with one `write(2)` per line, and not through `System.err`, so a
program that replaces `System.err` or `System.out` cannot redirect it.

A program can take descriptor 2 away: `System.err.close()` (or `FileDescriptor.err.close()`) makes the JDK point descriptor 2
at `/dev/null`, and a report written there would be lost without a trace. The JVM cannot duplicate a descriptor, so at start
the SDK opens descriptor 2 a second time, through `/proc/self/fd/2` (`/dev/fd/2` where there is no `/proc`) in append mode,
and writes to that second descriptor only once `FileDescriptor.err` has become invalid. Until then every line goes through
descriptor 2, like the program's own writes. The second descriptor is opened only when descriptor 2 is **not a regular
file**, for two reasons:

* Reopening a path gives the access the path allows, not the access descriptor 2 had. A JVM started with standard error
  closed (`2>&-`) gets its first open file, a JDK file opened read-only, as descriptor 2; reopening it for writing would
  write the report into the JDK.
* On a regular file (`2>file`) the second descriptor appends at the end instead of sharing the program's write offset, so
  the program's own later writes to `System.err` would overwrite the report.

So: with standard error a pipe or a terminal, a program that closes `System.err` still delivers the report. With standard
error a regular file, or with a program that closes descriptor 2 by other means (native code) or that started the JVM with
standard error closed, a program that closes `System.err` loses the report (the line writes fail and `end` is missing); the
program still exits normally. Unlike the C, Rust and Python SDKs there is no private copy kept at a high descriptor number.
If neither path can be opened (standard error is a socket, there is no descriptor directory) the SDK uses descriptor 2 only.

## What is written, and when

The lines and their widths are those of the C SDK, so see [c-sdk.md](c-sdk.md#what-is-written-and-when) for the full
account. Every line is `HAIRSPRING_EVENT ` plus one JSON object, at most 4096 bytes, written with one `write(2)`: each line is
one call of `FileOutputStream.write`, which on a descriptor is one system call for a line this size. Counters are
right-aligned in 20 characters, witnesses are padded to 197 characters and thread keys to 64, so a line's length does not
depend on any counter or on whether a failure happened.

**Everything is written at the end**: `hello`, then one `catalog` line per distinct (id, kind) pair, one `assert` per id,
`work`, one `thread` line per thread (up to 256), then `end`. At most once per process, from a shutdown hook or `report()`,
and only from the process that started the SDK. As in the Python SDK (and unlike the C and Rust SDKs, which send `hello`
and the catalog at startup), `hello` carries the catalog's hash, which Java cannot know before the sites have run. The
consequence is that **a process killed before it reports (a timeout, `SIGKILL`, `Runtime.halt`, a crash) leaves no lines
at all**. The engine treats a process without an `end` as `report_missing`; with no line at all it has nothing to say
about the process.

Declarations and evaluations past 16,384 distinct ids are not recorded and are counted in `end.dropped`, one per
evaluation.

### Shutdown

The report is written by a shutdown hook thread (`hairspring-report`), which runs when the last non-daemon thread ends,
when `System.exit` is called, or when the JVM receives `SIGTERM` or `SIGINT`. It does not run for `Runtime.halt` or
`SIGKILL`.

* **An explicit `report()` from a daemon thread is not cut off.** The JVM does not wait for daemon threads at exit, but the
  shutdown hook waits (up to 5 seconds) for a report another thread is still writing, so the lines are whole. A report
  that takes longer than that (a reader that has stopped reading a full pipe) can still be cut off.
* **Other threads keep running while the hook runs.** A non-daemon thread still evaluating, or a daemon thread, is not
  stopped. The hook takes one snapshot under the SDK's lock, so the report is consistent, and an evaluation that
  lands after the snapshot is not in it. A program that wants every evaluation counted joins its threads before it
  returns from `main`.
* **`System.exit` called from inside a shutdown hook blocks for ever** (the JVM waits for the hooks to finish). Use
  `Hairspring.exit` from ordinary code only.
* **An uncaught exception** in `main` ends the JVM with status 1; the hook still runs, and `end.status` says 0.
* A hook that fails (it cannot write, or throws) prints one line, `hairspring: the report could not be written: ...`, on
  standard error, which a parser ignores; the missing `end` line is what the engine sees.

### Threads

A fixed table of 256 slots, claimed under the SDK's lock on a thread's first SDK call (the slot is cached in a
`ThreadLocal`). Slots are never recycled and keys cannot be reused: the table is for fixed pools. Past the 255th thread
the rest share the last slot (`t#overflow`, `key_stable:false`) and each is counted in `end.dropped`. Ordinals in
witnesses are per thread, as in the C SDK. Every platform thread counts, including pool threads and the JVM's own
threads that call the SDK; **virtual threads each claim a slot**, so a program that starts many of them reaches the
overflow slot quickly. Use platform threads for code that asserts.

### Processes

Java cannot change its own environment, and cannot fork. A child started with `ProcessBuilder` inherits the environment
the JVM was started with, so a child that also uses this SDK would repeat its parent's tag. Call `Hairspring.prepare` on the
builder before `start()`:

```java
Process child = Hairspring.prepare(new ProcessBuilder(command).inheritIO()).start();
```

The child then finds its parent's tag and a pid that is not its own, takes `<parent>.s<pid>`, and says `tag_stable:false`
in its `hello`; it reports at its own exit. A child started without `prepare` reports under the tag the JVM was given
(`p1` unless `HAIRSPRING_PROC` was set), which collides with its parent's. There is no fork handler.

## Thread safety

Every change to a counter happens with one monitor held, so counts are exact under any number of threads. The fast path of
a call is one `ConcurrentHashMap` read (to find the site by the id as the caller spelled it), one `ThreadLocal` read, and
the monitor.

* A report is written from a **snapshot** taken under the monitor, so every count, witness and thread total belongs to one
  moment. A witness is only ever written behind a non-zero count (the parser rejects a line that pairs them). With one
  kind per id, the evaluations the thread lines add up to equal the hits the `assert` lines add up to in every snapshot,
  because both are changed inside one critical section. That equality is **not** promised for an id used with two kinds
  (the `assert` line carries only the first kind's counts, so the other kind's evaluations are in the thread totals but not
  in any `assert` line). The tests take snapshots while four threads evaluate and check the single-kind case, and take the
  lock out to confirm they fail without it.
* The critical sections are a few field updates. The report's lines are written outside the monitor.
* The site table registers new sites under the monitor; looking a site up never takes it.

## Call-site cost

A call does this: check the number of `vals`; find the site in a per-kind map; find the thread's slot in a `ThreadLocal`;
take the monitor; bump the thread's evaluation count and the site's `trues` or `falses`; on the first evaluation of a side,
store the witness (one small array). It allocates only what the caller passes: the `long...` array, which the JIT may or
may not remove. It is a lock acquire and a map read, and **it is not constant time with respect to the condition**: a program
under test that relies on the *absence* of a thread switch inside a window must keep SDK calls out of it (the fixture does).

## The planted race

`examples/language-fixtures/java` builds `lang-at.jar`, one program (`langat.Main`) with three modes chosen by its first
argument:

* `buggy`: 4 platform threads behind a start barrier, each doing 1,000 iterations of: load a shared plain `long` field, run
  a busy loop of 1,000 rounds (`Race.spin`, in the program's own source), store the value read plus one. The load and the
  store are separate operations, so an update is lost whenever another thread runs between them. There is no system call,
  allocation, lock, SDK call, sleep or yield between the load and the store; the SDK's work counter and the busy loop's
  result (a volatile field, which keeps the loop from being dropped) are written after the store.
* `fixed`: the same three steps inside `synchronized`.
* `wide`: `buggy` with a busy loop ten times longer.

An optional second argument, `--k=N` (1 to 5,000, plain digits), sets the iterations per worker for any mode; without it
K is 1,000 and the run is unchanged. Anything else is a usage error (exit 64, nothing on stdout). Only K changes: the
workers, the window and the format of the stdout line stay, and with `--k=N` the line's `expect=` and the property's
expected value are 4 x N. The cap is there for the JIT: the worker loop was measured compiled at K = 10,000 to 20,000
(see below), which would remove the window. See `examples/language-fixtures/README.md`.

Properties: `java.counter_exact` (`always`, the final counter equals 4 x K, 4,000 without `--k`; witness values
`[final, 4*K, 4, K]`, `[final, 4000, 4, 1000]` without `--k`) and
`java.all_started` (`sometimes`), each evaluated once in the main thread after the workers are joined. Output: one line on
stdout, `LANGAT v1 lang=java mode=<mode> final=<20 digits> expect=<20 digits> order=<16 hex> rand=<16 hex>`, written with one
call on `FileOutputStream(FileDescriptor.out)`, and exit 0 whatever the property says (3 for an internal failure, 64 for a
usage error). `rand` is the first value of a new `java.util.Random`.

**The window and the JIT.** Nothing in the language stops a compiler from moving a plain load past a loop or keeping a plain
field in a register, either of which would remove the window; HotSpot can only do that to code it compiles. It compiles a
method (or a loop in it, by on-stack replacement) when the method's invocation and loop back-edge counters pass a threshold,
and those counters are per method, summed over all threads: `racyIterations` is called 4 times and its loop passes
4 x 1,000 = 4,000 times in all, below the thresholds (about 60,000 back-edges for C1 at tier 3, about 10,700 with
`-XX:-TieredCompilation`). That is a margin, so `scripts/check-java-compiled.sh` runs every mode under
`-XX:+PrintCompilation`, with the default flags and under each profile, and requires that neither worker loop is ever
compiled. The two flag profiles in `examples/language-fixtures/java/profiles.tsv` are `-Xint ...` (interpreter only) and
`-XX:TieredStopAtLevel=1 ...` (C1 only); `profiles-extra.tsv` in the same directory adds `a-prime`, profile `a` with `-Xshare:off`
replaced by `-Xshare:auto`, which `make at-compiled` checks the same way. `scripts/check-window-java.sh` reads the bytecode of the two worker loops and
requires a load of the counter, one call of `spin` and a store of the counter with nothing else that calls, locks or
allocates between them. `spin` itself is a call inside the window: under arm b a worker can wait for a compile of it, and under
both arms a worker can stop at a safepoint in its loop; both only make the race easier to reach. The native self-check, run
with the default flags and under each profile's flags, is what shows the race fires.

Natively (macOS arm64, Temurin 21.0.5, default flags) `buggy` lost an update in 50 of 50 runs, `fixed` in 0 of 50 and `wide`
in 10 of 10; under both flag profiles `buggy` lost an update in 5 of 5 runs and `fixed` in none. These say the race exists
and the detector sees it, not how often it fires under any other scheduler.

## Conformance

* `cargo test -p hairspring-props --test java_sdk` compiles the SDK, the samples and the fixture and folds what they write
  with the reference parser. It mirrors the C SDK's `c_sdk` tests (complete undegraded report, a false `always` is a
  violation with its witness, an unmet `reachable`, merged sites, 100 and 300 threads, exit status, two processes on one
  output, a spawned child's tag and `tag_stable`, a catalog conflict, an empty catalog, limits), adds the events file,
  `HAIRSPRING_EVENTS=off`, a closed reader, `System.exit` and an uncaught exception (the hook still reports), `Runtime.halt`
  (it does not), a daemon thread evaluating during shutdown, and the planted race under both flag profiles. `HS_JAVA` names
  the `java` command (and `HS_JAVAC` the compiler if it is not next to it); without them `$JAVA_HOME/bin/java` and `java` on
  `PATH` are tried. With none, or one older than 21, the tests print a skip notice and pass, unless `HS_REQUIRE_JAVA` is set,
  which makes a missing JDK an error (CI sets it).
* `the_report_matches_the_c_sdks_byte_for_byte` runs the C SDK's `basic` scenario and the Java `basic` sample, which has the
  same ids, messages, values and order (the three sites the C program never reaches are `declare`d). The `assert`, `work`,
  `thread` and `end` lines are equal character for character (including every padding space); `hello` and `catalog` are
  equal once the SDK name and version and the source locations are removed; and the `catalog_hash` is equal, which also
  checks the id hash. This needs `make` and a C compiler, as the C SDK's own conformance tests do.

## Tests

`make -C sdks/java test` (JDK only, no framework):

* `hairspring.SelfTest`: unit tests of the formatter, every line kind, the catalog hash (including the value the C SDK
  prints), the thread slots, the site table and argument checks, the process identity rules, the `AT_SECURE` decision, the
  output channel (events file, symlink, FIFO with no reader, device, directory, missing directory, privileged process,
  failing stream) and the recorder (report shape and order, witnesses, merging, conflicts, declared sites, validation, exactness
  under eight contending threads, snapshot consistency while four threads run, the report once, thread caps, child
  environment). The tests were written first; the lock was taken out to check that the contention and snapshot tests fail
  without it.
* `test/test-process.sh`: the sample programs as real JVMs (exit statuses, `System.exit`, an uncaught exception, `halt`,
  100 and 300 threads, an explicit report, a daemon thread at shutdown, limits, 64 sites, a catalog conflict, a spawned child,
  the events file, a symbolic link, events off, a closed stderr pipe, a harness tag, a usage error).

The fixture has its own tests (`make -C examples/language-fixtures/java at-test`): its command line (including `--k=N`),
its line, its barrier, its invariants and its workers; the source check; the native tests of `--k=N`; 50 native runs per
mode with a negative control; and each JVM flag profile. The native targets take an optional `AT_K=<n>`
(`make -C examples/language-fixtures/java at-native AT_K=50`), which runs the program at `--k=<n>` and checks for 4 x n; without
it they run exactly as before.

## Differences from the C SDK

* `hello` and the catalog are written at the end, with the report (see above).
* Ids and messages are checked at the first evaluation (`IllegalArgumentException`) instead of at compile time.
* A site that never runs is invisible unless it is `declare`d.
* No `HAIRSPRING_SIGNAL_REPORT` (a fatal signal without a report shows as `report_missing`), no `HS_EVAL_*` per-evaluation
  lines, no fork handler, no `hs_choice` or tape, no call-site audit (there is no linker section to fall out of: every
  evaluation registers its own site). The C SDK's catalog is built from linker sections in the executable, which a JVM cannot
  load, so the SDK is not a binding to it.
* No `dup` of descriptor 2 (the SDK reopens it through `/proc/self/fd/2` or `/dev/fd/2` instead, see above) and no `SIGPIPE` mask: the JVM ignores `SIGPIPE`, and a write to a reader that has gone
  shows as an `IOException`, which costs the line (counted in `end.dropped`), not the process.
* The events file is opened for appending without following a link and must be a regular file, but Java has no
  `O_NONBLOCK` for files: a FIFO is refused by looking at it before opening it, and a FIFO swapped in between the two steps
  is not caught. A write to a full non-blocking pipe on standard error loses the line.
* `AT_SECURE` is only consulted on Linux (see "Environment").
* Linux and macOS only.

## Not done

* A JVM agent (`-javaagent`) that would catalog every call site before it runs, instead of `declare`.
* Signal-time reports, `HS_EVAL_*` equivalents, a fork handler, the tape API.
* Windows.
* Hermit: supported on a Java-specific configuration only (`skid_margin = 35`, interpreter-only JVM flags, a reduced fixture, slow trials), and a replay matched only from the same paths as the recording (JDK install, program jar and working directory). See the [support matrix](support-matrix.md) and [Replay and paths](../sdks/java/README.md#replay-and-paths).
