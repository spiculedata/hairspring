# .NET SDK

The .NET SDK (`sdks/dotnet`, assembly `Hairspring`, namespace `Hairspring`) lets a .NET program declare assertions that
Hairspring tracks across trials. It speaks [guest protocol v1](guest-protocol.md), writes the same lines as the
[C SDK](c-sdk.md) (checked byte for byte, see "Conformance"), is managed C# for `net10.0` using only the base class library
(no NuGet package, no agent, no native code of its own), supports Linux and macOS, and is not published to a registry.

> **Status.** The SDK, its tests and a lost-update fixture exist in this repository. A .NET program has **not yet been run
> under Hermit**, and **.NET is not supported on this build**. Everything below was checked natively (Linux and macOS, no
> Hermit); nothing here says how a .NET program behaves under the `process` backend.

A call site only updates counters in memory; the program reports once, when the process exits.

```csharp
using static Hairspring.Hs;

Always(balance >= 0, "bank.balance_non_negative", "no overdraft", [balance]);
Sometimes(waited, "bank.writer.waited", "a writer waited for the lock");
Reachable("bank.recovery.ran", "recovery ran");
Unreachable("bank.bad_state", "never expected");
Work(1);                       // operations done, reported as work.ops
NameThread("writer", 0);       // stable thread key "writer-0"
```

Build it with `make -C sdks/dotnet dll` (`Hairspring.dll`), or reference `sdks/dotnet/src/Hairspring.csproj` from your project.
A runnable program is `sdks/dotnet/samples/Sample`.

> **A site that never runs is invisible, not unmet.** .NET cannot see a call site before it executes, so a `Reachable`,
> `Sometimes` or `Always` in code the run never reaches is simply absent from the report, and the run cannot flag it as
> UNMET. That is the main thing those kinds exist to find. **Call `Declare(Kind.Reachable, id, msg)` once, early, for every
> site whose absence matters.**

> **The SDK starts when its class is first used.** Any call starts it, or call `Hs.Start()`. A program that never touches
> the class reports nothing, not even a `hello`. **Call `Hs.Start()` first thing in `Main`** if the first use of the SDK
> could be on a thread that the end of the process can cut off (a background thread): the exit report is registered when the
> SDK starts, and a process that is already ending can finish before a late start has registered it.

## API

All methods are static on `Hairspring.Hs`. `vals` is a `ReadOnlySpan<long>` (write it as a collection expression, `[a, b]`; no
array is allocated).

| Method | Meaning | Trial-level failure | Run-level UNMET |
|---|---|---|---|
| `Always(cond, id, msg, vals)` | must hold at every evaluation | any false | never evaluated |
| `AlwaysOrUnreachable(cond, id, msg, vals)` | same, but never reaching it is fine | any false | none |
| `Sometimes(cond, id, msg, vals)` | must hold at least once per run | none | never true |
| `Reachable(id, msg, vals)` | must be reached at least once per run | none | never hit |
| `Unreachable(id, msg, vals)` | must never be reached | hit | none |

`msg` and `vals` are optional. Each method also takes `[CallerFilePath]` and `[CallerLineNumber]` parameters, which the compiler
fills in; do not pass them.

| Method | Meaning |
|---|---|
| `Declare(kind, id, msg)` | puts a site in the catalog without evaluating it. **Needed for any site that might never run** |
| `Work(n)` | adds `n` to the calling thread's operation count, reported as `work.ops` (a total over all threads; the sum stops at the largest value instead of wrapping) |
| `NameThread(label, index)` | gives the calling thread the key `label-index`. Returns `false` if another thread has the key or the thread shares the overflow slot, and the thread then keeps its claim-order key `t#<n>`, flagged `key_stable:false`. Throws `ArgumentException` for an empty label, a label with a NUL character or a key over 64 bytes, and `ArgumentOutOfRangeException` for an index outside `0..4294967295`. Call it before the thread does anything else |
| `Report()` | writes the report now (reason `explicit`); the first report wins, and evaluations after it are not reported |
| `Exit(status)` | records the status and calls `Environment.Exit(status)`, so the exit report carries it |
| `Prepare(startInfo)` | puts the process tag in the environment of a child you are about to start (see "Processes") |
| `Start()` | starts the SDK without evaluating anything |

* `cond` is a `bool`. `vals` is 0 to 4 values, captured at the first true and the first false evaluation (`vals` in the
  protocol; missing values are 0). A fifth value throws `ArgumentException` and counts nothing.
* `id` is 1 to 128 bytes of UTF-8; bytes outside `[A-Za-z0-9._:/-]` are sent as `_` (as in the C SDK), so ids that differ only
  in such bytes are one id. `msg` is at most 512 bytes; bytes that are not printable ASCII (so every control, format and
  bidirectional character, and every byte of a non-ASCII character) are sent as `?`, and `"` and `\` are escaped, as in the C
  SDK. An empty or over-long id, an id containing a NUL character, or an over-long message throws `ArgumentException` (the C
  SDK refuses these at compile time; C# has no compile time for them). A `null` id or message throws `ArgumentNullException`.
  Nothing is recorded for the call. **The message is checked on every call**, also for a site that is already known.
* The same id and kind from several places is one site: the report merges them into one `assert` line, and `end.sites` counts
  distinct ids. An id used with two kinds is a catalog conflict the engine reports; the `assert` line carries the first
  kind's counts.
* The location in the `catalog` line is the first call's file and line, from the compiler (`CallerFilePath`,
  `CallerLineNumber`): nothing is looked up at run time. Only the last two parts of the path are sent (`src/Race.cs`), so a
  build machine's directories do not leave the machine.
* `end.status` is the status the process exits with: the value `Exit(n)` or `Environment.Exit(n)` was given, the value
  `Main` returned, or the status `Environment.ExitCode` held. A program that crashes is reported as a signal (see
  "Shutdown").

## Environment

| Variable | Effect |
|---|---|
| `HAIRSPRING_EVENTS_FILE=path` | write events to this file (appended; it must be a regular file; a link, a FIFO, a device or a directory is refused) instead of standard error; if it cannot be used, events go to standard error and `end.dropped` counts it. Ignored in a set-uid or set-gid process (`AT_SECURE` from `/proc/self/auxv` on Linux, where an unreadable vector counts as privileged; `issetugid` on macOS, where a failure to ask counts as privileged) |
| `HAIRSPRING_EVENTS=off` | record as usual but write nothing |
| `HAIRSPRING_PROC=tag` | the root process tag: 1 to 64 bytes of `[A-Za-z0-9._:/-]`, not `_eval`; default `p1`. An invalid tag is refused (the default is used and `end.dropped` is incremented), never shortened |
| `HAIRSPRING_PROC_PID` | set for children by `Prepare`; do not set it |
| `HAIRSPRING_SIGNAL_REPORT` | **not supported**: ignored, and `hello.signal_handlers` is always false |

### The events file

The file is looked at without following links first (a FIFO, a device, a directory or a link, even a dangling one, is refused
before anything is opened for writing). A missing file is created exclusively (`O_EXCL`, mode `rw-r--r--` before the umask).
It is then opened for appending with `O_NOFOLLOW`, `O_NONBLOCK` (a FIFO swapped in at the last moment cannot hang the program),
`O_NOCTTY` and close-on-exec, and looked at again through the descriptor: one that is not a regular file, or not the file first
seen, is closed and refused. `O_APPEND` makes two processes that share the file write whole lines one after the other.

### Standard error

The report goes to standard error with `write(2)` on descriptor 2 itself, one call per line, and not through `Console.Error`,
so a program that replaces `Console.Error` or `Console.Out` cannot redirect it. (The runtime's console stream writes to a
duplicate of the descriptor made when the console is first used, which is not the descriptor a harness or a trace looks at.)

At start the SDK also makes a private duplicate of descriptor 2 (the same open file, so it shares the offset and the append
mode; close-on-exec; the lowest free descriptor number). Each line is checked before it is written:

* descriptor 2 is still open for writing and is still the file it was at start (device and inode compared): the line goes
  through descriptor 2, like the program's own writes;
* descriptor 2 was closed (native code can do that; `Console.Error.Close()` does not: it was checked that descriptor 2 stays open, because the console works on its own
  duplicate), is no longer open for writing, or has been replaced by a different file: the line goes to the private
  duplicate, so the report is not lost and is not written into whatever file took the number.

A file is **never reopened by name**, so the SDK never takes an access the descriptor did not have. If descriptor 2 is not open
for writing at start (the program was started with standard error closed, `2>&-`, and the runtime's first open file, which
may be read-only, got the number; or standard error is a file opened read-only), nothing is written to it: the lines fail, the
report is missing its `end`, and the program exits normally. If the file system cannot tell what descriptor 2 is (a system
call is filtered), descriptor 2 is used as long as it stays open for writing.

A write that fails (a reader that has gone, `EPIPE`; a full disk) costs that line, not the process: the runtime ignores
`SIGPIPE`. The line is counted in `end.dropped`. A write that fails after the kernel took part of a line (a full disk, a
non-blocking pipe) can leave a torn line, and the next line is then glued to it, so the parser drops both.

Limits of the descriptor handling: if the program was started with standard error closed and the runtime's first open file
became descriptor 2 and happens to be writable (a pipe or socket of the runtime's own), the report is written into it; the
private duplicate is made with `dup` and then marked close-on-exec in a second call, so a child started in between could
inherit it; and the duplicate takes the lowest free number, which a program that counts descriptors may notice.

## What is written, and when

The lines and their widths are those of the C SDK, so see [c-sdk.md](c-sdk.md#what-is-written-and-when) for the full account.
Every line is `HAIRSPRING_EVENT ` plus one JSON object, at most 4096 bytes, written with one `write(2)`. Counters are
right-aligned in 20 characters, witnesses are padded to 197 characters and thread keys to 64, so a line's length does not
depend on any counter or on whether a failure happened. Numbers are formatted the same under every culture.

**Everything is written at the end**: `hello`, then one `catalog` line per distinct (id, kind) pair, one `assert` per id,
`work`, one `thread` line per thread (up to 256), then `end`. At most once per process, from the exit event or `Report()`, and
only from the process that started the SDK (the process id is asked of the kernel each time, because `Environment.ProcessId`
is cached: a process forked from a .NET program never reports). As in the Python and Java SDKs (and unlike the C and Rust
SDKs, which send `hello` and the catalog at startup), `hello` carries the catalog's hash, which .NET cannot know before the
sites have run. The consequence is that **a process killed before it reports (a timeout, `SIGKILL`, `_exit`, a crash that
raises no managed event) leaves no lines at all**. The engine treats a process without an `end` as `report_missing`; with no
line at all it has nothing to say about the process.

Declarations and evaluations past 16,384 distinct ids are not recorded and are counted in `end.dropped`, one per evaluation;
`end.sites` never says more than 4,096, the most a parser accepts, and the report of such a process is degraded.

### Shutdown

The report is written from the `AppDomain.ProcessExit` event. What each way of ending a process does (checked on Linux and
macOS with runtime 10.0.x):

| How the process ends | Reported | `end.reason`, `end.status` |
|---|---|---|
| `Main` returns (any return type) | yes | `exit`, the returned value |
| `Environment.Exit(n)` or `Hs.Exit(n)` | yes | `exit`, `n` |
| an unhandled exception | yes, from `AppDomain.UnhandledException`, before the runtime aborts | `signal`, 6 (`SIGABRT`) |
| `Environment.FailFast` | no (the runtime runs no handler) | |
| `SIGTERM`, `SIGINT` | no: with no handler of the program's own the runtime ends the process by the signal, without the exit event (a program that handles the signal and then ends normally is reported) | |
| `SIGKILL`, `_exit`, a timeout kill | no | |

Other fatal signals were not tried.

* **An explicit `Report()` from a background thread is not cut off.** The runtime does not wait for background threads at exit,
  but the exit event waits (up to 5 seconds) for a report another thread is still writing, so the lines are whole. A report
  that takes longer than that (a reader that has stopped reading a full pipe) can still be cut off.
* **Other threads keep running while the exit event runs.** A foreground thread still evaluating, or a background thread, is
  not stopped. The event takes one snapshot under the SDK's lock, so the report is consistent, and an evaluation that lands
  after the snapshot is not in it. A program that wants every evaluation counted joins its threads before it returns from
  `Main`.
* **`Environment.Exit` runs the exit event, with the status it was given.** Call it from ordinary code, not from an exit
  handler.
* A report that fails (it cannot write, or throws) prints one line, `hairspring: the report could not be written: ...`, on
  descriptor 2, which a parser ignores; the missing `end` line is what the engine sees.

### Threads

A fixed table of 256 slots, claimed under the SDK's lock on a thread's first SDK call (the slot is cached in a
`ThreadLocal`). Slots are never recycled and keys cannot be reused: the table is for fixed pools. Past the 255th thread the
rest share the last slot (`t#overflow`, `key_stable:false`) and each is counted in `end.dropped`. Ordinals in witnesses are
per thread, as in the C SDK. **Every managed thread that calls the SDK takes a slot, each thread-pool thread included, and a
`Task` or an `await` continuation runs on whichever pool thread is free**, so a program that asserts from many tasks reaches
the overflow slot quickly and its thread keys differ between runs. Use dedicated `Thread`s, named with `NameThread`, for code
that asserts.

### Processes

A .NET program cannot change its own environment for a child it starts through the operating system, and cannot fork. A
child started with `Process.Start` inherits the environment the process was started with, so a child that also uses this SDK
would repeat its parent's tag. Call `Hs.Prepare` on the start info before starting it:

```csharp
using var child = Process.Start(Hs.Prepare(new ProcessStartInfo("dotnet", "child.dll") { UseShellExecute = false }));
```

The child then finds its parent's tag and a pid that is not its own, takes `<parent>.s<pid>`, and says `tag_stable:false` in
its `hello`; it reports at its own exit. A child started without `Prepare` reports under the tag the process was given (`p1`
unless `HAIRSPRING_PROC` was set), which collides with its parent's. There is no fork handler: the runtime does not support
forking a managed program, and the SDK only guarantees that a forked copy writes nothing (the owner check above). On Linux,
with the runtime's write-xor-execute memory on (its default), the parent of the fork test crashed; with
`DOTNET_EnableWriteXorExecute=0` it ran, so the fork test sets that.

## Thread safety

Every change to a counter happens with one monitor held, so counts are exact under any number of threads. The fast path of a
call is one `ConcurrentDictionary` read (to find the site by the id as the caller spelled it), one `ThreadLocal` read, and the
monitor.

* A report is written from a **snapshot** taken under the monitor, so every count, witness and thread total belongs to one
  moment. A witness is only ever written behind a non-zero count (the parser rejects a line that pairs them). With one kind
  per id, the evaluations the thread lines add up to equal the hits the `assert` lines add up to in every snapshot, because
  both are changed inside one critical section. That equality is **not** promised for an id used with two kinds (the `assert`
  line carries only the first kind's counts, so the other kind's evaluations are in the thread totals but not in any `assert`
  line). The tests take snapshots while four threads evaluate and check the single-kind case.
* The critical sections are a few field updates, and **nothing outside the SDK is called inside them**: the report's lines are
  written, and the sink is called, with no lock held (a test blocks the writer inside the sink and checks that other threads
  can still record, snapshot and name themselves).
* The site table registers new sites under the monitor; looking a site up never takes it.

## Call-site cost

A call does this: check the number of `vals` and the message; find the site in a per-kind map; find the thread's slot in a
`ThreadLocal`; take the monitor; bump the thread's evaluation count and the site's `Trues` or `Falses`; on the first
evaluation of a side, store the witness (one small array). It allocates nothing on a known site except at the first
evaluation of each side. It is a lock acquire and a map read, and **it is not constant time with respect to the condition**: a
program under test that relies on the *absence* of a thread switch inside a window must keep SDK calls out of it (the fixture
does). A message of more than 170 characters is measured in UTF-8 bytes on every call (no allocation).

## The planted race

`examples/language-fixtures/dotnet` builds `LangAt.dll` (with `LangAt.runtimeconfig.json`, `LangAt.deps.json` and `Hairspring.dll`
beside it), one program with three modes chosen by its first argument (`dotnet LangAt.dll buggy|fixed|wide`):

* `buggy`: 4 threads behind a start barrier, each doing 1,000 iterations of: load a shared plain `long` field, run a busy loop
  of 1,000 rounds (`Race.Spin`, in the program's own source, marked `NoInlining`), store the value read plus one. The load and
  the store are separate operations, so an update is lost whenever another thread runs between them. There is no system call,
  allocation, lock, SDK call, sleep or yield between the load and the store; the SDK's work counter and the busy loop's result
  (a static field) are written after the store.
* `fixed`: the same three steps inside `lock`.
* `wide`: `buggy` with a busy loop ten times longer.

Properties: `dotnet.counter_exact` (`Always`, the final counter equals 4,000, witness values `[final, 4000, 4, 1000]`) and
`dotnet.all_started` (`Sometimes`), each evaluated once in the main thread after the workers are joined. Output: one line on
stdout, `LANGAT v1 lang=dotnet mode=<mode> final=<20 digits> expect=<20 digits> order=<16 hex> rand=<16 hex>`, written with one
`write(2)` on descriptor 1, and exit 0 whatever the property says (3 for an internal failure, 64 for a usage error). `rand` is
`Random.Shared.NextInt64()`, which is never negative, so its top bit is always 0.

**The window and the JIT.** `Spin` is marked `NoInlining`, so the window holds an ordinary call between the load and the store
of the field, and a compiler has to assume that a call it did not inline can write the field. That is the design; it was **not
verified in the generated code**: the machine code of the two worker loops was not read, and there is no check that the loops
are never recompiled (the Java fixture has one for its interpreter and compiler tiers; this one has not). What was checked is
the behaviour: the native self-check, run in the default environment and under both environment profiles of
`examples/language-fixtures/dotnet/profiles.manifest` (arm b turns tiered compilation off, so everything is compiled with
optimisations at once), sees the race fire. These say the race exists and the detector sees it, not how often it fires under
any other scheduler.

Natively (macOS arm64 and Linux arm64 with runtime 10.0.x) `buggy` lost an update in 50 of 50 runs, `fixed` in 0 of 50 and
`wide` in 10 of 10, in the default environment and under both profile arms.

## Conformance

* `cargo test -p hairspring-props --test dotnet_sdk` (40 conformance tests) builds the samples and the fixture and folds what they write with the
  reference parser. It mirrors the C SDK's `c_sdk` tests (complete undegraded report, a false `always` is a violation with its
  witness, an unmet `reachable`, merged sites, 100 and 300 threads, exit status, two processes on one output, a spawned
  child's tag and `tag_stable`, a catalog conflict, an empty catalog, limits), adds the events file (and a link refused),
  `HAIRSPRING_EVENTS=off`, a closed reader, a program that closes descriptor 2, standard error a file, `Environment.Exit`, a
  status returned from `Main`, an unhandled exception (reported as `SIGABRT`), `_exit` (no report), a forked copy (writes
  nothing), a background thread evaluating during shutdown, refused arguments, and the planted race under the second
  environment profile. `HS_DOTNET` names the `dotnet` command; without it `$DOTNET_ROOT/dotnet` and `dotnet` on `PATH` are
  tried. With none, or one older than 10, the tests print a skip notice and pass, unless `HS_REQUIRE_DOTNET` is set, which
  makes a missing SDK an error (CI sets it).
* `the_report_matches_the_c_sdks_byte_for_byte` runs the C SDK's `basic` scenario and the .NET `basic` sample, which has the
  same ids, messages, values and order (the three sites the C program never reaches are `Declare`d). The `assert`, `work`,
  `thread` and `end` lines are equal character for character (including every padding space); `hello` and `catalog` are equal
  once the SDK name and version and the source locations are removed; and the `catalog_hash` is equal, which also checks the
  id hash. This needs `make` and a C compiler, as the C SDK's own conformance tests do.

## Tests

`make -C sdks/dotnet test` (.NET SDK only, no framework, no package):

* `Hairspring.SelfTest` (161 cases): unit tests of the formatter (culture independence, control and bidirectional
  characters), every line kind, the catalog hash (including the value the C SDK prints), the thread slots, the site table and
  argument checks, the process identity rules, the `AT_SECURE` decision, the system calls (what a path or descriptor is,
  whole-line writes, a closed reader, close-on-exec, appending without following links), the events file (a link, a dangling
  link, a FIFO with and without a reader, a device, a directory, a missing directory), standard error (valid, closed,
  replaced, read-only, never open, a regular file shared with the program's own writes, a reader that has gone) and the
  recorder (report shape and order, witnesses, merging, conflicts, declared sites, validation on every call, exactness under
  eight contending threads, snapshot consistency while four threads run, the report once, the owner check, thread caps, the
  writer wait, no lock held while writing, the site limit, child environment). The tests were written first (the self-test
  was committed red, before the SDK existed).
* `test/test-process.sh` (69 checks): the sample programs as real processes (exit statuses, `Environment.Exit`, an
  unhandled exception, `_exit`, 100 and 300 threads, an explicit report, a background thread at shutdown, a forked copy,
  limits, 64 sites, refused input, a catalog conflict, a spawned child, the events file with a link, a dangling link, a FIFO,
  a device and a directory, events off, a closed stderr pipe, a read-only and a closed descriptor 2, a harness tag, a usage
  error).

The fixture has its own tests (`make -C examples/language-fixtures/dotnet at-test`): its command line, its line, its barrier,
its invariants and its workers; the source check; 50 native runs per mode with a negative control, in the default
environment and under each profile arm.

## Differences from the C SDK

* `hello` and the catalog are written at the end, with the report (see above).
* Ids and messages are checked at each evaluation (`ArgumentException`) instead of at compile time.
* A site that never runs is invisible unless it is `Declare`d.
* No `HAIRSPRING_SIGNAL_REPORT` (a fatal signal without a report shows as `report_missing`), no `HS_EVAL_*` per-evaluation
  lines, no fork handler, no `hs_choice` or tape, no call-site audit (there is no linker section to fall out of: every
  evaluation registers its own site). The C SDK's catalog is built from linker sections in the executable, which a managed
  program has none of, so the SDK is not a binding to it.
* The SDK calls the C library through P/Invoke for `write`, `dup`, `fcntl`, `ioctl`, `open`, `statx` or `fstat`, `getpid`
  and `issetugid`, and `open`, `fcntl` and `ioctl`, which are variadic in C, are called only in forms that take no variadic argument (`open`
  without `O_CREAT`, `fcntl(F_GETFL)`, `ioctl(FIOCLEX)`), because Apple arm64 passes variadic arguments differently. On
  Linux the name `libc` is resolved to `libc.so.6`, so a system without the development package still works; on a system
  where neither name loads (musl was not tried) the SDK starts as a recorder that writes nothing. A system call that is not there (a filtered `statx`) fails like any other error
  and the SDK carries on without that check, except for the events file, which is then refused.
* The private duplicate of descriptor 2 uses the lowest free descriptor number, not a high one: `fcntl(F_DUPFD_CLOEXEC, 10)`
  is a variadic call. It is close-on-exec.
* Linux and macOS only. On another system the SDK starts as a recorder that counts and writes nothing.
* Not tested: Native AOT and trimming (the P/Invoke names are plain, but this was not tried), musl, and macOS on x86-64 (the
  `$INODE64` entry points are used for `stat` there).

## Not done

* A profiler or startup hook that would catalog every call site before it runs, instead of `Declare`.
* Signal-time reports, `HS_EVAL_*` equivalents, a fork handler, the tape API.
* Windows.
* Nothing here ran under Hermit: the planted race has not been reached through the engine, how the runtime's threads, clocks
  and JIT behave under the process backend is not known, and **.NET is not supported on this build**.
