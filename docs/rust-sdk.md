# Rust SDK

The Rust SDK (`sdks/rust`, crate `hairspring-sdk`) lets a Rust program declare assertions that Hairspring tracks across
trials. It speaks [guest protocol v1](guest-protocol.md), writes the same lines as the [C SDK](c-sdk.md) (checked byte
for byte, see "Conformance"), has no dependencies, and is not published to a registry.

A call site only updates counters in memory; the program reports once, at exit.

```rust
use hairspring_sdk::{always, reachable, sometimes, thread, unreachable_point, work};

always!(nbackfill <= mxframe, "wal.ckpt.nbackfill_le_mxframe", "nBackfill <= live mxFrame", nbackfill, mxframe);
sometimes!(waited, "wal.writer.waited", "a writer waited for the lock");
reachable!("wal.reset.reached", "the WAL was reset");
unreachable_point!("wal.bad.state", "never expected");
work(1);                          // operations done, reported as work.ops
let _ = thread("writer", 0);      // stable thread key "writer-0"
```

Add it to a workspace member or a path dependency: `hairspring-sdk = { path = "sdks/rust" }`. A runnable example is in
`sdks/rust/examples/quickstart.rs`.

## API

| Macro | Meaning | Trial-level failure | Run-level UNMET |
|---|---|---|---|
| `always!(cond, id, msg, ...)` | must hold at every evaluation | any false | never evaluated |
| `always_or_unreachable!(cond, id, msg, ...)` | same, but never reaching it is fine | any false | none |
| `sometimes!(cond, id, msg, ...)` | must hold at least once per run | none | never true |
| `reachable!(id, msg, ...)` | must be reached at least once per run | none | never hit |
| `unreachable_point!(id, msg, ...)` | must never be reached | hit | none |

| Function | Meaning |
|---|---|
| `work(n)` | adds `n` to the process's operation count, reported as `work.ops` |
| `thread(label, index)` | gives the calling thread the key `label-index`; `Err(KeyError)` if the label is empty, the key is over 64 bytes, another thread has it, or the thread shares the overflow slot. The thread then keeps its claim-order key `t#<n>`, flagged `key_stable:false`. Call it before the thread does anything else |
| `report()` | writes the report now (reason `explicit`); the first report wins |
| `exit(status)` | records the status and exits through the normal path, so the report carries it |
| `init()` | starts the SDK now and makes sure it is linked into a program with no assertion of its own |

* `cond` is a `bool`. `...` is 0 to 4 values of any type that casts to `i64`, captured at the first true and the first
  false evaluation (`vals` in the protocol; missing values are 0). A fifth value is a compile error.
* `id` and `msg` must be string literals (or `concat!` of literals) and are **checked at compile time**: an id is 1 to
  128 bytes of `[A-Za-z0-9._:/-]`; a message is at most 512 bytes of printable ASCII without `"` or `\`. Anything else
  is a compile error (`evaluation panicked: an id may only contain ...`; the check functions are unit tested, and a
  bad id was seen to fail a build by hand, but there is no automated compile-fail test). (The C SDK replaces bad bytes at run time instead. The Rust macros refuse them, so the offline
  catalog below can always be valid JSON and ids never merge by accident.)
* The same id at several call sites is fine (a macro in a generic function, an inlined function, two functions): the
  report merges them into one `assert` line, and `end.sites` counts distinct ids. An id used with two kinds is a catalog
  conflict the engine reports. A `static` is one item however a generic function is instantiated or inlined (the
  `generic` sample checks this), so the C SDK's g++ template problem does not arise.
* `unreachable_point!` is not called `unreachable!` so that it cannot shadow the standard library's panicking macro.
* On glibc the exit handler is registered with `on_exit`, so a plain `return` from `main` or `process::exit(n)` is
  reported as `n`. With musl or on macOS an exit handler cannot see the status and `end.status` is 0 unless `exit` was
  used. **`end.status` is advisory.** The engine always uses the operating system's wait status.
* A call site that runs before the SDK has started (another constructor, a static initialiser) starts it itself, inside
  the call site's slow path. That first call is therefore not syscall-free. Normally a constructor starts the SDK
  before `main`.

## Environment

| Variable | Effect |
|---|---|
| `HAIRSPRING_EVENTS_FILE=path` | write events to this file (`O_APPEND`, `O_NOFOLLOW` so a symlink is refused, `O_NONBLOCK`, and it must be a regular file) instead of stderr; if it cannot be opened or is not a regular file, events go to stderr and `end.dropped` counts it. Ignored in a set-uid or set-gid process (`AT_SECURE` on Linux, `issetugid` on macOS, or differing real and effective ids) |
| `HAIRSPRING_EVENTS=off` | record as usual but write nothing |
| `HAIRSPRING_PROC=tag` | the root process tag: 1 to 64 bytes of `[A-Za-z0-9._:/-]`, not `_eval`; default `p1`. An invalid tag is refused (the default is used and `end.dropped` is incremented), never shortened |
| `HAIRSPRING_PROC_PID` | set by the SDK; do not set it |
| `HAIRSPRING_SIGNAL_REPORT` | **not supported**: ignored, and `hello.signal_handlers` is always false |

At startup the SDK duplicates descriptor 2 (or opens the events file) to a high number with
`fcntl(F_DUPFD_CLOEXEC)` and writes only to that copy, so a program that closes stderr and then opens a data file
cannot receive the report. It remembers which file the copy refers to and checks it with one `fstat` before the exit
report. As in the C SDK this is best effort: programs under test must not `dup2` onto or close that descriptor while
running. The [C SDK page](c-sdk.md#descriptors) has the longer discussion; it applies unchanged.

## What is written, and when

The lines and their widths are those of the C SDK, so see [c-sdk.md](c-sdk.md#what-is-written-and-when) for the full
account. In short:

1. **Startup** (a constructor, before `main`): `hello`, then one `catalog` line per distinct (id, kind) pair, so a
   program that crashes, hangs or is killed still has a catalog.
2. **Exit** (an exit handler or `report()`; at most once per process): one `assert` per distinct id, `work`, one `thread`
   per thread (up to 256), then `end`.

Every line is `HAIRSPRING_EVENT ` plus one JSON object, at most 4096 bytes, written with one `write(2)` in the normal
case. Counters are right-aligned in 20 characters, witnesses are padded to 197 characters and thread keys to 64, so a
line's length does not depend on any counter or on whether a failure happened.

### Threads

A fixed table of 256 slots, claimed on a thread's first SDK call. Slots are never recycled and keys cannot be reused:
the table is for fixed pools. Past the 255th thread the rest share the last slot (`t#overflow`, `key_stable:false`) and
each is counted in `end.dropped`. A slot is reported only once its key is published.

### Processes

The tag travels in the environment (`HAIRSPRING_PROC`, `HAIRSPRING_PROC_PID`), exactly as in the C SDK. A child started
with `std::process::Command` inherits the parent's tag and a pid that is not its own, so it takes `<parent>.s<pid>` and
says `tag_stable:false` in its `hello`. An image that `exec`s without forking takes `<tag>.x1`. **There is no fork
handler** (Rust programs rarely `fork` by hand): a raw `libc::fork` child shares the parent's SDK state and reports
nothing of its own. The SDK records the pid of the process that started it and writes the report only from that
process, so a child that exits does not repeat its parent's report (`hs_sample_fork`). Anything a forked child
evaluates is lost.

## The catalog is collected by the linker

Each call site declares its record (id, message, source location, kind, and the live counters) as a function-local
`static` placed in a named section. The linker concatenates the records, and the SDK reads the whole section back as a
slice at startup. Nothing registers at run time.

| | Linux (ELF) | macOS (Mach-O) |
|---|---|---|
| Records | section `hs_rust_catalog_l1` | `__DATA,__hs_rs_cat_l1` |
| Offline JSON twins | section `hs_rust_catstr_l1` | `__DATA,__hs_rs_cstr_l1` |
| How the SDK finds the bounds | the linker defines `__start_hs_rust_catalog_l1` and `__stop_hs_rust_catalog_l1` for a section whose name is a C identifier | ld64 defines `section$start$__DATA$__hs_rs_cat_l1` and `section$end$__DATA$__hs_rs_cat_l1` for any section that exists |
| Keeping records through garbage collection | `#[used]`; the records are also referenced by the code that evaluates them | the same |

* **Both need the section to exist.** The SDK always plants one sentinel record (kind `None`) in each section, and the
  reader refers to the sentinel so the object that holds it is linked. A program with no assertion at all but a call to
  `init()` or `work()` therefore links and reports an empty catalog (`hs_sample_empty`).
* **The names differ from the C SDK's** (`hs_catalog_v1`, `__hs_catalog`) on purpose. The record layouts differ, and a C
  and a Rust SDK linked into one executable must not read each other's records. The wire output of the two is unaffected.
* **Records are never mutated.** The C SDK writes its index into the records at startup; here they are immutable
  apart from atomic counters and witnesses, and the index (canonical record per id, which records get a `catalog`
  line, the hash) is built in ordinary memory.
* **Offline catalog.** Each call site also places the JSON of its `catalog` line (without `proc`) in the twin section as
  a NUL-terminated string, so a tool can read the catalog from the binary without running it. A test checks that the
  texts in a built binary (found by scanning the file, not by parsing sections) equal the live `catalog` lines. A reader
  must skip runs of NUL bytes and must not assume the first text of the section starts at a NUL: on x86-64 Linux the
  first one directly followed the bytes of the preceding section. The twin is built at compile time with the same
  escaping and trimming as the live line (the file path keeps its last 256 bytes; `"` and `\` are escaped; other
  non-printable bytes become `?`), and unit tests compare the two through the formatter for paths with quotes,
  backslashes, non-ASCII bytes and more than 256 bytes. It is always valid JSON (the C SDK cannot promise that).
* **Order.** Records come back in link order, which is not source order on every platform (on macOS the example
  `basic` sample lists `basic.always.bad` last). Reports do not depend on order: the catalog hash is order independent.
* **Witnesses are not torn.** A witness is written by one thread (the one whose counter left zero) with `ord` stored
  last, with release, and read with acquire, so a report racing live workers never prints an ordinal with the values
  of another write. A stress test checks 200,000 witnesses read while they are written. (On aarch64 the release store
  is a store-release instruction rather than a plain store.)

### Limits worth knowing

* **Thread keys can be read torn.** `thread` rewrites a slot's key while the slot may already be live (it has a
  claim-order key from its first call), and a report on another thread at that moment can print a mixed key. The
  window is a few byte stores; call `thread` before the thread does anything else.
* **A `#[global_allocator]` must not call into the SDK.** Startup allocates (the identity, the index), and a call site
  that runs inside the allocator before the SDK has started would start it from inside an allocation. Call `init()`
  early if your allocator is instrumented, or keep call sites out of it.
* **The events descriptor must not be redirected by the program** (see "Environment").

### What happens when the sections do not work

| Situation | Result | Checked |
|---|---|---|
| A program never references the crate | the crate is not linked and nothing is reported (there is nothing to report); call `init()` to force the link | by construction |
| A call-site record that is not in the section (a hand-built `Site` in a plain `static`, which no macro produces) | its evaluations are counted by the thread but not attributed to any catalogued record; at report time the SDK sees the totals disagree, adds one to `end.dropped` and writes one plain-text line `hairspring: incomplete catalog: <n> evaluation(s) ran at call sites that are missing from the catalog section` to the events descriptor | yes: `hs_sample_audit`, positive control |
| A linker that does not define the bounds symbols | a link error (an undefined `__start_` or `section$start$` symbol), never a silent loss | not tested (GNU ld, lld and Apple ld64 define them) |
| The SDK linked into a shared object or `cdylib` | not supported. By construction the shared object has its own copy of the SDK, with its own sections and state, and the bounds symbols resolve inside it, so it reads its own records; but two copies in one process each report under the same tag (a duplicate `hello` and `end`, so the trial is degraded) and each starts its own constructor and exit handler. Unlike the C SDK, nothing stays unreported because of a section nobody reads. This reasoning is not tested | not tested |
| Two copies of the crate in one executable (two versions) | the section names carry a layout number (`_l1`), so copies with different layouts do not read each other's records; copies with the same layout would share the section and both report all of it, with a duplicate tag. Do not do this | not tested; a test pins the layout so a change must bump the number |
| An unsupported OS | a compile error (`compile_error!`): Linux and macOS only | by construction |

The audit has the same limits as the C SDK's, and the tolerance for in-flight evaluations is identical (one per thread
inside a call site between its two counter updates; see c-sdk.md). It is a detection, not a fix. The compile-time design
makes the failure it guards against far less likely here than in C++: a macro cannot create a record outside the
section, because the attribute is part of the macro.

## Call-site cost

Requirements: no system call, no allocation, no lock, no printing; the same work whether the condition holds or not. A
call site (`eval` in `src/eval.rs`) does this and nothing else:

1. find the thread's slot through a `const`-initialised thread-local pointer (a plain load: no lazy set-up and no
   destructor registration); if it is null (the thread's first SDK call) take the cold path, which claims a slot;
2. `ord = evals + 1` (a relaxed load and store of a thread-owned word);
3. `ok = cond as u64`, `bad = ok ^ 1`, an empty assembly block keeps the compiler from turning the masks into branches;
4. `fetch_add` on the record's `trues` and `falses` (relaxed, on the record's own cache line: they commute, so the
   totals do not depend on the interleaving);
5. the evaluation that moves a counter off zero owns that side's witness; the destination is chosen by arithmetic, and
   six stores go to it unconditionally (the real witness for the winner, the thread's private scratch for everyone else).

There is no compare-and-swap, so there is no retry loop. The one data-independent branch is the slot-null test.

**Evidence.** `hs_sample_nosyscall` (Linux, x86-64 and aarch64) installs a seccomp filter that kills the process on any
system call except `exit`, then runs a million evaluations of every kind, `work` and `thread` in a fresh thread, with an
allocation counter armed; the check passes only if nothing was killed and nothing allocated. The allocation counter
matters: a thread's own malloc arena can serve small allocations without a system call, so the filter alone would
miss an allocation in the slot claim or in `thread` (an earlier version of this code made one, and only the counter
caught it).

`examples/callsite.rs` compiles two call sites as plain functions. Reading the release build once (x86-64 with GNU ld
2.44, aarch64, rustc 1.99): neither function contains a `syscall` or `svc` instruction; the only conditional branch is
the slot-null test; the witness selection is `cmov`/`csel` and mask arithmetic, so the same instructions run for a true
and for a false condition; the thread-local access is one load from the thread pointer on Linux. The compiler chose
conditional moves where the C source uses masks; both are data-independent. These listings were read, not turned into a
test. On macOS the access is a call through `_tlv_get_addr`, as in the C SDK, so macOS is for development builds.

## The planted race

`sdks/rust/fixtures/lang_at/` builds one binary, `at-rust`, that follows a fixed output and mode contract shared by
the language fixtures. The mode is the first argument, never a build flag, so a single artifact runs in every arm:

* `at-rust buggy`: 4 threads, each doing 1,000 iterations of: load a shared `AtomicU64`, spin 1,000 busy iterations (an
  opaque accumulator behind `black_box` in a non-inlined function; no system call in the window), store the value
  loaded plus one. A separate load and store is well defined and still loses updates.
* `at-rust fixed`: the same, with a `Mutex` held around each load, spin and store.
* `at-rust wide`: `buggy` with a window ten times longer (10,000 busy iterations); nothing else differs.
* Any other argument, none, or more than one: a usage line on stderr and exit 64.
* Start barrier: each worker registers its stable thread key (`thread("worker", i)`), increments a shared `started`
  counter, and waits until all four have started before its first iteration. A per-iteration `work(1)` follows the
  store, outside the window.
* Properties, evaluated once each in the main thread after the workers are joined: `always!(final == 4000,
  "rust.counter_exact")` with the witness `[final, 4000, 4, 1000]`, then `sometimes!(started == 4,
  "rust.all_started")`.
* Output: one stdout line, written with one write, `LANGAT v1 lang=rust mode=<mode> final=<20 digits> expect=<20
  digits> order=<16 hex> rand=<16 hex>` (`order` is the FNV-1a 64 hash of the finishing order written as four digits;
  `rand` is one value from the runtime's random source, so that entropy leaks show). Exit 0 whatever the property says:
  the verdict comes from the report. Exit 3 is an internal invariant failure (a thread did not start or finish, the
  finishing order is incomplete).
* It does not read `HAIRSPRING_SEED`, `/proc`, the CPU count or the clock. This needed no change to the SDK crate.

Native self-tests (`sdks/rust/tests/fixture_race.rs`, and the same through the reference parser in
`hairspring-props/tests/rust_sdk.rs`) show that the detector can fire in `buggy` and `wide` (a lost update, reported as
`always` false with the witness `[final, 4000, 4, 1000]`, within 200 native runs) and never fires in `fixed` (20 runs,
every one exactly 4000), and check the stdout line, the exit statuses and that each property is evaluated once. The
binary has unit tests for the mode parser, the line format, the order hash and the workers. They prove only that the
race exists and the report sees it; nothing here ran under Hermit.

## Conformance

* `cargo test -p hairspring-props --test rust_sdk` builds the SDK's sample programs with a nested `cargo build` and folds
  what they write with the reference parser (`HS_RUST_SDK_BUILD=<dir>` uses prebuilt programs instead). It mirrors the C
  SDK's `c_sdk` tests (complete undegraded report, an `always` false is a violation with its witness, an unmet
  `reachable`, merged sites, 100 and 300 threads, exit status, two processes on one output, a spawned child's tag and
  `tag_stable`, a catalog conflict, an empty catalog, limits) and adds the planted race.
* `the_report_matches_the_c_sdks_byte_for_byte` runs the C SDK's `basic` scenario and the Rust `basic` sample, which has
  the same ids, messages, values and order. The `assert`, `work`, `thread` and `end` lines are equal character for
  character (including every padding space); `hello` and `catalog` are equal once the SDK name and version and the
  source locations are removed; and the `catalog_hash` is equal, which also checks the id hash.
  This needs `make` and a C compiler, as the C SDK's own conformance tests do.

## Tests

`cargo test -p hairspring-sdk` (it needs no toolchain beyond Rust):

* unit tests of the formatter, every line kind, the catalog index and merge (including the hash value the C SDK
  prints), the audit arithmetic, the thread slots (300 claims, 32 simultaneous first claims, key rules), the process
  identity rules, and the compile-time checks;
* `tests/events.rs`: the shape of a report, the events file (and a symlink), `HAIRSPRING_EVENTS=off`, process tags, exit
  status, merged ids, 100 and 300 threads, and the audit positive control;
* `tests/offline_catalog.rs`: the twins in a built binary equal the live `catalog` lines;
* `tests/fixture_race.rs`: the language fixture's three modes, its output line and exit statuses;
* `tests/no_syscall.rs`: the seccomp check (it prints `skip` and passes on macOS).

Where it has been run: macOS arm64 (debug and release); Linux aarch64 natively in a container (debug with GNU ld; release
with `lld` 19); Linux x86-64 in a container under emulation (release with fat LTO and GNU ld, everything except the
seccomp check, which cannot install its filter under that emulation). The seccomp check has therefore run on aarch64
Linux only; CI's native x86-64 runner is its first x86-64 run.

## Differences from the C SDK

* Ids and messages are compile-time checked instead of sanitised at run time.
* No `HAIRSPRING_SIGNAL_REPORT` (a crash without a report shows as `report_missing`), no `HS_EVAL_*` per-evaluation
  lines, no fork handler (`<parent>.f<n>` tags), no `hs_choice` or tape (neither exists in the C SDK yet either).
* Linux and macOS only. Like the C SDK, the SDK blocks `SIGPIPE` around the startup lines and the report and consumes
  a `SIGPIPE` that its own write raised, so a reader that has gone away costs the report, not the process. This matters
  before `main`: Rust ignores `SIGPIPE` only once its runtime has started, and the startup lines are written from a
  constructor (`a_closed_stderr_pipe_does_not_kill_the_program_with_sigpipe`).
* Writing the events file: it is opened non-blocking and must be a regular file. A FIFO with no reader fails to open at
  once and one with a reader is refused after opening; either way events fall back to stderr and `end.dropped` counts
  it, where the C SDK would block in `open` until a reader appears (an open issue for C). The check is made on the file
  that was opened, so it does not race with a path swap.
* Section names differ, and the records are immutable (see above).
* The Linux `O_NOFOLLOW` value is chosen by architecture (x86, x86-64 and generic: `0o400000`; arm, aarch64 and powerpc:
  `0o100000`). Other architectures are untested.

## Not done

* Signal-time reports, `HS_EVAL_*` equivalents, a fork handler, the tape API.
* More than 4,096 ids is untested for this SDK (the parser's handling is covered by the C SDK's `manyids` scenario).
* `hairspring catalog <binary>` (the tool that reads the twin sections) does not exist yet.
* The seccomp check has not run on native x86-64 Linux here.
* No run under Hermit; the planted race has not been reached through the engine.
