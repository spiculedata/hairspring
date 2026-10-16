# C SDK

The C SDK (`sdks/c`) lets a C program declare assertions that Hairspring tracks across trials. It speaks
[guest protocol v1](guest-protocol.md). A call site only updates counters in memory; the program reports once, at
exit (or from a fatal signal, if the target opts in).

```c
#include "hairspring.h"

HS_ALWAYS(nbackfill <= mxframe, "wal.ckpt.nbackfill_le_mxframe", "nBackfill <= live mxFrame", nbackfill, mxframe);
HS_SOMETIMES(waited, "wal.writer.waited", "a writer waited for the lock");
HS_REACHABLE("wal.reset.reached", "the WAL was reset");
HS_UNREACHABLE("wal.bad.state", "never expected");
HS_WORK(1);                       /* operations done, reported as work.ops */
hs_thread("writer", 0);           /* stable thread key "writer-0" */
```

## API

| Macro | Meaning | Trial-level failure | Run-level UNMET |
|---|---|---|---|
| `HS_ALWAYS(cond, id, msg, ...)` | must hold at every evaluation | any false | never evaluated |
| `HS_ALWAYS_OR_UNREACHABLE(cond, id, msg, ...)` | same, but never reaching it is fine | any false | none |
| `HS_SOMETIMES(cond, id, msg, ...)` | must hold at least once per run | none | never true |
| `HS_REACHABLE(id, msg, ...)` | must be reached at least once per run | none | never hit |
| `HS_UNREACHABLE(id, msg, ...)` | must never be reached | hit | none |
| `HS_WORK(n)` | adds `n` to the process's operation count | | |

* `id` and `msg` must be string literals: they are pasted into the offline catalog at compile time. Ids are 1 to 128
  bytes of `[A-Za-z0-9._:/-]`; other bytes are sent as `_`. Messages keep printable ASCII only (other bytes become
  `?`), at most 512 bytes.
* `...` is 0 to 4 integer values, captured at the first true and the first false evaluation (`vals` in the protocol;
  missing values are 0).
* Using the same id at several call sites is fine (macros inside inline functions, headers included twice): the
  report merges them into one `assert` line, and `end.sites` counts distinct ids.
* `hs_thread(label, index)` gives the calling thread the key `label-index`. It returns -1 (and the thread keeps its
  claim-order key `t#<n>`, flagged `key_stable:false`) if the label is empty or too long, or the key is already taken.
* `hs_report()` writes the report now (reason `explicit`); `hs_exit(status)` records the exit status and exits.
  **`end.status` is advisory.** The engine always uses the operating system's wait status for how a process ended.
  With glibc the exit handler is registered with `on_exit`, so a plain `exit(n)` or `return n` from `main` is reported
  as `n`; with musl or on macOS an exit handler cannot see the status and `end.status` is 0 unless `hs_exit` was used.
* A call site that runs before the SDK's constructor (another constructor, a static initialiser) does the startup
  itself inside the call site's slow path (`hs__claim` calls `hs_init`); that first call is therefore not syscall-free.
* The macros are checked at compile time: an id of 129 bytes or more, a message over 512 bytes and more than four
  values are errors (`tests/fixtures/fail_*.c`). They work from C++ (`-std=gnu++11` or later; values of any integer or
  enum type). Call sites in C++ templates need care: see "C++ templates and the catalog section".
* `HS_EVAL_ALWAYS(cond, id)`, `HS_EVAL_SOMETIMES`, `HS_EVAL_ALWAYS_OR_UNREACHABLE`, `HS_EVAL_REACHABLE`,
  `HS_EVAL_UNREACHABLE` write one protocol `eval` line per call. That is a `write(2)` at the call site, so the engine
  marks the trial `perturbing` and excludes it from tier claims. They record nothing in memory and need no catalog.
* `hs_choice` (the tape API) is not implemented yet: the tape layout is not agreed. `thread.tape_consumed` is
  always 0 until it exists.

Build: `make -C sdks/c lib` makes `build/<cc><opt>/libhairspring.a`; compile your program with
`-Isdks/c/include`, link the archive and `-pthread`. Link statically into the executable: the catalog is found
through the executable's own section.

**Shared objects are not supported.** Link the SDK into the executable only. If an executable and a shared object each
link a copy, the dynamic linker resolves both copies' state to the executable's (the symbols are not hidden), the
second constructor finds the SDK already initialised, and the call sites inside the shared object are in a section
nobody reads: they count, but are never catalogued or reported. Nothing detects this at run time. For the same reason
the thread-local slot pointer is declared `initial-exec` on Linux (one load from the thread pointer, no
`__tls_get_addr`), which a library loaded with `dlopen` cannot use.

### C++ templates and the catalog section

Each call site declares its record as a function-local `static` with a section attribute, and the catalog is the
linker's collection of that section. For code in a C++ function template (or a member of a class template) the
compiler instantiates that static once per instantiation, as a COMDAT (vague-linkage) entity.

* **What is guaranteed.** With clang (Apple clang, clang++ 18 on Linux x86-64) the records of template call sites are
  in the catalog section, and the tests hold it to that (`make cxx-templates` requires it whenever `$(CC) --version`
  names clang). With any compiler, an evaluation made at a call site whose record is *not* in the section is detected
  at report time, unless one of the limits under "What is not detected" applies; it is not a guarantee against every
  loss.
* **What is observed with g++ 13.3 on Linux x86-64** (issue #23): at `-O0` and `-O2`, the records and
  offline strings of template call sites were emitted into per-instantiation COMDAT sections
  (`.data.rel.local.<mangled>`, `.rodata.<mangled>`) instead of `hs_catalog_v1` / `hs_catalog_str_v1`. Those sites were
  absent from the `catalog` and `assert` lines and from the offline catalog. A call site in a non-template `inline`
  function was placed correctly, so the difference is the template. Why g++ does not apply the section attribute to the
  template's static is **not established**: it is inferred, not verified, to be a compiler behaviour for statics in
  template instantiations. Other g++ versions are untested here.
* **g++ 13.3 and mixed files** (observed on Linux x86-64 in two layouts: the earlier fixture and `cxx_mixed.cc`): a file that has a call site in an `inline`
  function **and** a call site in an ordinary (non-inline, non-template) function does not compile; g++ reports
  `'hs__site' causes a section type conflict with 'hs__site'` (and the same for `hs__offline`). Template sites were
  not part of that error. Inference, not verified: g++ puts the static of an inline function into the named section as
  a COMDAT-group member and the static of an ordinary function into the same-named section without a group, and
  rejects one section name with different flags in a file. A file with only inline and template sites compiles, and
  so does a file with only ordinary sites. `make cxx-mixed` checks the observed behaviour per compiler (g++ must fail
  with that message, clang must compile and catalogue both sites); it is not part of `make test` because it asserts
  compiler behaviour. Member functions defined inside a class body are implicitly `inline`, so a call site in a
  non-template class's in-class member function that shares a file with a call site in an ordinary function is the
  same mix and is expected to fail under g++ the same way (inference; not tested).
* **Advice.** For C++ use clang, which catalogues every kind of site above. On Linux x86-64 with clang 18.1.3
  the mixed file compiled and both sites were catalogued at `-O0` and `-O2` (also on Apple clang). With g++, keep call sites in ordinary
  (non-inline, non-template) functions; sites in templates compile (in a file without the inline/ordinary mix) but
  land outside the section, and the audit announces them once they are evaluated.
* **What is detected.** Every evaluation made through a call-site macro adds one to the calling thread's `evals` and
  one to its record's `trues` or `falses`. At report time the SDK compares the sum of the threads' `evals` with the sum
  of the counters of the records it can find in the section. If the threads counted more, the difference is the number
  of evaluations that happened at call sites outside the catalog. The SDK then adds one to `end.dropped` and writes a
  single plain-text line (no `HAIRSPRING_EVENT` prefix, so a protocol parser ignores it) to the events descriptor,
  which is stderr unless `HAIRSPRING_EVENTS_FILE` names a file:
  `hairspring: incomplete catalog: <n> evaluation(s) ran at call sites that are missing from the catalog section`.
  A program whose stderr is compared exactly therefore sees one extra line when this happens. Call sites inside a
  shared object are in the same position (their records are in a section nobody reads), so the audit should flag them
  too; shared objects remain unsupported and this was not tested.
* **What is not detected.** The limits, all of which can hide a loss:
  * A call site that was never evaluated in the run, and whose record is outside the section, leaves no trace: it is
    absent from the catalog and nothing counts it.
  * With several threads the check is deliberately lenient. A thread counts an evaluation in its slot a few
    instructions before it counts it in the record, so the totals can differ by one per thread that is inside a call
    site at that moment, and the SDK cannot tell finished threads from running ones. It therefore ignores a shortfall of
    up to the number of claimed slots minus one at exit or after `hs_report` (the reporting thread is not inside a call
    site), and up to the number of claimed slots after a fatal signal. With one thread every missing evaluation is
    reported (after a signal, a single missing evaluation is forgiven). In a program that has used many threads, a
    small loss can go unnoticed.
  * The audit is skipped once 255 or more threads have claimed a slot (the shared last slot's count is not exact).
  * **Weakly ordered CPUs (for example aarch64).** The call site uses relaxed atomics, so on such a CPU the update of
    `evals` and the update of the record are not guaranteed to be observed in program order by the reporting thread.
    The "at most one evaluation in flight per thread" bound used above may then not hold, and a multi-threaded exit
    could be reported as an incomplete catalog although nothing is missing. This is reasoning from the memory model,
    not something that has been observed; on x86-64 the stores are not reordered this way.
  * **A report that interrupted a call site.** A report started from a signal handler (the SDK's own handler after a
    fatal signal, or a program's handler calling `hs_report` or `exit`), or a `longjmp` out of a handler, that
    interrupted a thread between its two counter updates can be flagged falsely. After a fatal signal the SDK's own
    handler allows for the interrupted thread (see above); a program's own handler that reports or exits, or a
    `longjmp` out of a handler, is not covered.
  * With `HAIRSPRING_EVENTS=off` nothing is written, so there is no diagnostic line; `end.dropped` is still counted
    in the process but is not reported either, since the `end` line is not written.
  * The offline catalog (`hs_catalog_str_v1`) has no such check: a binary read without running it shows only the sites
    that were placed in the section.
* **This is a detection, not a fix.** Registering a record from inside a function body would, as far as is known here,
  need a run-time initialisation check at the call site (for example a guarded function-local static with a dynamic
  initialiser), which the cost argument above rules out; no such design was built or measured. Another route, also
  untried, is to emit the record from inline assembly with `.pushsection` / `.popsection` so the compiler's section
  handling is bypassed; whether that would work for template and inline sites, and at what cost, is unknown. Keep the
  assertions that must be reported in non-template code, or in a function the template calls, or check the run for
  `end.dropped` / the diagnostic line.

## Environment

| Variable | Effect |
|---|---|
| `HAIRSPRING_EVENTS_FILE=path` | (ignored in a set-uid or set-gid process: `secure_getenv` on glibc 2.17 or newer, `issetugid` on macOS and the BSDs; on any other libc the SDK reads it with plain `getenv`, so a set-uid process still honours it there) write events to this file (`O_APPEND | O_NOFOLLOW`, so a symlink is refused) instead of stderr; if it cannot be opened, events go to stderr and `end.dropped` counts it |
| `HAIRSPRING_EVENTS=off` | record as usual but write nothing |
| `HAIRSPRING_SIGNAL_REPORT=1` | install SIGSEGV, SIGBUS, SIGABRT, SIGFPE, SIGILL and SIGTERM handlers that write the report (reason `signal`, status = the signal number) and re-raise; `hello.signal_handlers` is then true only if every handler was installed. A signal that already has a non-default action is left alone and counts as not installed: an inherited `SIG_IGN` (`trap '' TERM`, `nohup`) stays ignored, and a handler a sanitizer or another runtime put there stays in place (under AddressSanitizer, SIGSEGV and SIGBUS are such signals, and on macOS SIGFPE too). Default off: handlers add syscalls at startup and change how a crash looks under Hermit |
| `HAIRSPRING_PROC=tag` | the root process tag: 1 to 64 bytes of `[A-Za-z0-9._:/-]`, not `_eval`; default `p1`. An invalid tag is refused (the default is used and `end.dropped` is incremented), never shortened |
| `HAIRSPRING_PROC_PID` | set by the SDK (see below); do not set it |

### Descriptors

At startup the SDK duplicates descriptor 2 (or opens the events file) to a high number with `fcntl(F_DUPFD_CLOEXEC)`
and writes only to that copy, so a program that closes stderr and then opens `data.db` (which becomes descriptor 2)
cannot receive the report. This is one extra syscall at startup (two when the first high number is refused), which
the T3 "A" arm must also make. The SDK also remembers which file the copy refers to (`st_dev`, `st_ino`) and checks it with one `fstat` before the
exit report and before every `eval` line (so each `eval` line costs one extra syscall; the identity is recorded before
the descriptor is published, so a second thread cannot see it half set up). A program that `dup2`s something onto the
copy's number, or closes every descriptor, loses the events from then on: the SDK writes nothing and counts it in
`end.dropped` (which a lost report cannot show; the run's `report_missing` does).

This check is best effort, not a guarantee. The check and the `write` are separate steps, so a program thread that
`dup2`s a file onto the descriptor between them can still receive one event line. The same holds while a long exit
report is being written line by line (one check, many writes), and in the low-`RLIMIT_NOFILE` fallback below when
another thread closes and reopens descriptor 0, 1 or 2. A reviewer reproduced this with a thread flipping the
descriptor in a loop (10 of 200 runs received a line). User space cannot close the window, so programs under test must
not redirect, close or reuse the events descriptor while running.

With a very low `RLIMIT_NOFILE` no private copy can be made; the SDK then falls back
to the original descriptor, still exposed to the program, and counts that in `end.dropped`. If stderr is already
closed at startup there is no output. The copy is close-on-exec: an exec'd image makes its own copy from its inherited
descriptor 2.

### Processes: fork, exec and spawn

Every process in a trial needs a unique tag. The SDK keeps its tag in the environment, as `HAIRSPRING_PROC` and the
owner's pid in `HAIRSPRING_PROC_PID`, in static buffers it hands to `putenv`; a fork child rewrites them in place
(no allocation), so what a child execs inherits a tag nobody else uses.

| How the process starts | Tag |
|---|---|
| the root process | `HAIRSPRING_PROC` if the harness set it, else `p1` |
| `fork()` (the `pthread_atfork` handler runs) | `<parent>.f<n>`, `n` counting the parent's forks; if that passes 64 bytes, `<first 40 bytes>.h<16 hex digits>` |
| `fork()` then `exec` of an instrumented image | the same `<parent>.f<n>`, inherited through the environment; the image sends its own `hello`, `catalog`, `assert`s and `end` once |
| `fork()` then `exec` of an uninstrumented image | nothing: a fork child sends `hello` and `catalog` only when it reports, so a child that never reports leaves no trace |
| `posix_spawn`, `system()`, `vfork`-style starts (no fork handler) | the inherited pid is not the child's own, so the child takes `<inherited>.s<pid>`. A pid differs between runs outside Hermit, so such a tag, and every tag below it, is announced with `hello.tag_stable:false` (additive; the parser ignores and counts it) |
| `exec` of another image **without** forking | the image inherited its own pid with the "announced" flag set, so it takes `<tag>.x1`; the old image is gone and never sends `end`, so the trial shows it as `report_missing` |

Limits, stated plainly:

* A forked child that **crashes with signal handlers off** never reports, so it is invisible (no `hello`, no
  `catalog`); the parent's report is unaffected. With handlers on it reports `reason: signal`.
* `setenv`/`unsetenv` of `HAIRSPRING_PROC` by the program breaks the in-place rewrite; do not. A program that replaces
  or clears `environ` (`clearenv`, an `execve` with its own `envp`) does not export the tag at all: what it starts
  begins as `p1` again, duplicates its parent's tag, and the trial is degraded.
* **The backend must strip** any stray `HAIRSPRING_PROC` and `HAIRSPRING_PROC_PID` from the environment it gives the
  guest (or set `HAIRSPRING_PROC` itself and no `HAIRSPRING_PROC_PID`); a stray pair makes the first process look
  like a spawned child.
* `fork` takes the `hs_thread` key lock in its prepare handler and releases it in both the parent and the child, so a
  child never inherits the lock held by a thread that does not exist in it.
* Independently started instrumented programs (not children of an instrumented one) must be started by the harness
  with distinct `HAIRSPRING_PROC` values.

## What is written, and when

Every line is `HAIRSPRING_EVENT ` plus one JSON object, at most 4096 bytes including the newline, and is written with
**one** `write(2)` in the normal case (a short write is completed with further writes; see "Write path").

1. **Startup** (a constructor, before `main`): `hello`, then one `catalog` line per distinct id, so a program that
   crashes, hangs or is killed still has a catalog. A fork child sends its `hello` and `catalog` later, when it
   reports (see "Processes").
2. **Exit** (an exit handler, `hs_report`, or a fatal signal when opted in; at most once per process): one `assert` per
   distinct id, `work`, one `thread` per thread (up to 256), then `end`.

Properties of the report:

* **Fixed width.** Counters are right-aligned in 20 characters (`status` too), each witness (`first_false`,
  `first_true`) is padded to a constant 197 characters whether it holds an object or `null`, and a `thread` line's key
  is padded to 64 bytes, so a line's length does not depend on any counter, on whether a failure happened, or on which
  key a slot got. `key_stable` is padded to 5 characters. Both paddings are whitespace between JSON tokens, which the
  protocol allows. The cost: an `assert` plus its `catalog` line is about 850 bytes per id, whatever it says. The
  witness padding (197 characters) stays. The engine's per-trial event budget is **shared by every process of the
  trial**: at 1 MiB it holds about 1,200 ids summed over processes, and it is being raised to 4 MiB in the engine
  (about 4,900 ids).
* `reachable` and `unreachable` send `trues = hits`, `falses = 0` and only `first_true`.
* **One reading.** The report reads every thread's totals once; the `thread` lines, `end.threads` and `end.evals`
  all come from that reading, so threads still running (or starting) while the process exits cannot make them
  disagree. A slot is counted only once its key is published, so a thread that has claimed a slot but not yet named it
  is left out of that report. Counters of other threads may still advance while the report is written, and the report's
  own `write` calls interleave with them.
* **Threads.** A fixed table of 256 slots, claimed on a thread's first SDK call. **Slots are never recycled and keys
  cannot be reused**: the table is for fixed pools. A process that starts more than 255 threads over its lifetime
  shares the last slot (`t#overflow`, `key_stable:false`) among the rest, counts each such thread in `end.dropped`, and
  gets approximate counts in that slot under contention. `hs_thread` serialises its duplicate-key check with a spin
  lock (it is not a call site), so two threads asking for the same key cannot both get it.
* **Catalog hash.** `hello.catalog_hash` is an order-independent hash of the distinct (id, kind) pairs, including
  the second pair of an id declared with two kinds.
* **Conflicting kinds.** An id declared with two kinds gets a `catalog` line for each, so the engine sees the conflict;
  the first declaration (in section order) is the canonical one, and the other kind's counts are not merged into it.
  When several records of the same kind share an id, the merged witness is the earliest ordinal on one thread; witnesses
  from different threads cannot be ordered, and the first in section order stands.
* **Offline catalog.** Each call site also places the JSON of its catalog line (without `proc`) in the section
  `hs_catalog_str_v1` (Mach-O: `__DATA,__hs_catstr`), as NUL-terminated strings, so a tool can read the catalog from
  the binary without running it. A test checks that this text equals the live `catalog` line for the same site.
  `loc.func` is not available at compile time and is omitted. **`id` and `msg` must not contain `"` or `\`**: a C
  string literal cannot be escaped or inspected by the preprocessor, so the offline copy would be invalid JSON, and
  for an id that is outside the id alphabet the offline copy would differ from the live id (which has `_` for each
  disallowed byte). The runtime line escapes `msg` and replaces id bytes, so a run is unaffected.
  **Requirement for C2** (`hairspring catalog <binary>`): it must not depend on the twin being valid JSON. It has to
  either read the strings as fields and apply the same sanitising as the live line (escape `msg`, replace id bytes
  outside the alphabet, cut at the limits), or the SDK has to switch the twin to NUL-separated fields first.

### Limits and losses

Losses inside the SDK are counted in `end.dropped` (threads without a slot of their own, ids over the index, refused
settings, lines that did not fit or could not be written). The protocol's parser does not treat `dropped > 0` as
degradation by itself, and this branch does not change it; the engine should do so in a follow-up in `hairspring-props`.
Where the parser can notice a loss without that, the SDK makes it:

* More than 4096 distinct ids: the SDK still sends `catalog` and `assert` lines for all of them (up to 16384), so the
  parser counts the excess as over the limit and marks the trial degraded. `end.sites` is capped at 4096 to match what
  the parser accepted. In practice the per-trial event budget trips first (see above).
* Beyond 16384 distinct ids the extra records are not catalogued at all; only `end.dropped` shows it.
* An id outside the alphabet is sent with `_` in place of each bad byte; two such ids can merge.
* Evaluations at call sites that are not in the catalog section (some C++ template sites under g++, see "C++ templates
  and the catalog section") are counted in `end.dropped` by one and announced by a plain-text line; the sites
  themselves cannot be listed.

### Limits and caveats

Known limits of the SDK as built. None is hidden from the parser except where noted.

* **The alternate signal stack belongs to one thread.** With `HAIRSPRING_SIGNAL_REPORT=1` the constructor calls
  `sigaltstack` on the thread that runs it (normally the main thread). A stack overflow in any other thread has no
  alternate stack to run the handler on, so the process dies without a report (the run shows `report_missing`).
  Other faults in worker threads are reported (see the `threadsegv` test).
* **Thread keys can be read torn.** `hs_thread` rewrites a slot's key while the slot may already be live (it has a
  default `t#N` key from its first call). A report on another thread at that moment can print a mixed key. The
  window is one `memcpy` of at most 64 bytes; call `hs_thread` before the thread does anything else.
* **A witness can be read torn.** The first true and first false witness are written with plain stores by the thread
  that owns them, a moment after the counter moves. A report written concurrently (an `exit` on another thread, a
  signal) can print a witness whose fields come from two moments. Counters are exact; only a witness read during its
  own first hit is affected.
* **The offline catalog text is not escaped.** The strings in the `hs_catstr` section are the id and message as the
  preprocessor sees them, so they can hold quotes, backslashes or bytes outside the alphabet that the live line
  escapes or replaces. C2 has to escape and sanitise them (see "Requirement for C2" above). A reader of that section
  must also skip runs of NUL bytes: padding between strings is zero (under AddressSanitizer it is a redzone, which an
  unsanitised reader such as the extractor sees as zeros; the unit test walks it with `no_sanitize("address")`).
* **`end.threads` and `end.sites` are not padded.** Every other counter is a fixed-width field, but these two are
  written as plain decimals, so the `end` line is one to a few bytes longer or shorter depending on their values.
  The line is far under 4096 bytes whatever they are.
* **A non-blocking stderr can lose lines.** If the descriptor is `O_NONBLOCK` and the reader is slow, a write that
  returns `EAGAIN` is retried with `sched_yield` up to 1000 times, then the line is dropped and counted in
  `end.dropped`. A report can therefore be partial: some lines out, later ones missing (a missing `end` line shows
  as `report_missing`).

### Write path

* The report runs with SIGPIPE blocked (one `pthread_sigmask` before and one after), so a closed reader costs the
  report, not the process; a SIGPIPE that this write raised is consumed before the mask is restored. After a fatal
  signal the mask is left alone, since the process is about to die. Each `eval` line does the same around its own
  write. These are two `rt_sigprocmask` calls per report and per `eval` line for the T3 arms to replay. (`sigwait`,
  which consumes a SIGPIPE the write raised, is not on POSIX's async-signal-safe list; it is only reached outside the
  signal path.)
* `EINTR` is retried, `EAGAIN` is retried with `sched_yield` (both up to 1000 times per line), and a short write is
  completed with a second `write` of the remainder. If a line still fails after part of it went out, one newline is
  written so the next line does not glue onto the fragment (the parser counts the fragment as malformed). Every failed
  line increments `end.dropped`, and `errno` is restored.
* A pipe write larger than `PIPE_BUF` is not atomic. Completing a short write therefore gives up the interleaving
  guarantee for that line; with the sizes here it only happens on a signal or a full pipe.

### What differs from the plan's single-write design

The plan (sections 2.5, 2.6, 4.4) assumes a fixed-length startup write and a report formatted into one 256 KiB buffer
and written in 64 KiB chunks. The SDK writes **one line per `write(2)`** instead, to meet the 4096-byte pipe-atomicity
rule. For the T3 "A" arm (which replays the instrumented run's syscalls without the instrumentation) that means:

* Startup: 1 `hello` write plus one write per distinct id, then at exit one write per distinct id (`assert`), one
  `work`, one per thread slot in use, one `end`. Each write's length is fixed by the catalog (ids, messages, file
  names) and the number of slots in use, never by a counter value.
* Besides the writes: one `fcntl(F_DUPFD_CLOEXEC)` at startup (two if the first high number is refused; one `open`
  more with `HAIRSPRING_EVENTS_FILE`), one `fstat` before the report and one more before each `eval` line, two `rt_sigprocmask` around the report (and
  around each `eval` line), and with `HAIRSPRING_SIGNAL_REPORT=1` one `sigaltstack` and six `rt_sigaction` at startup.
* The exit report can interleave with threads that are still running, and they can add lines of their own (an
  `eval`) between the report's.

## Call-site cost

Requirements: no syscall, no allocation, no lock, no printing; the same work whether the condition holds or not.

A call site (`hs__eval` in `hairspring.h`) does this and nothing else:

1. find the thread's slot through a thread-local pointer; if it is null (the thread's first SDK call) call
   `hs__claim()`, a `fetch_add` on a counter plus plain stores;
2. `ord = ++slot->evals` (a plain load and store of a thread-local word);
3. `ok = (cond != 0)`, `bad = ok ^ 1`, an empty `asm` keeps the compiler from turning the masks into branches;
4. `prev_t = fetch_add(&site->trues, ok)`, `prev_f = fetch_add(&site->falses, bad)` (relaxed atomics on the site's own
   cache line; they commute, so the final totals do not depend on thread interleaving);
5. the evaluation that moved a counter off zero owns that side's witness: `win = side & is_zero(prev)`, where
   `is_zero(x) = ((x | -x) >> 63) ^ 1`. The destination is chosen by arithmetic, not a ternary:
   `dst = (site_witness & -win) | (slot_scratch & ~(-win))`. Six stores then go to `dst` **unconditionally** (slot
   index, ordinal, four values): the real witness for the winner, the thread's private scratch for everyone else.

There is no compare-and-swap, so there is no retry loop and no early exit on a failed compare. `hits` is not stored
(it is `trues + falses`). `reachable` and `unreachable` use the true side only.

What stays data dependent, and why it does not matter:

* The slot-null test is one well-predicted conditional branch. It depends on the thread's history (taken once per
  thread), never on the condition.
* The caller's `cond` expression is the caller's: `a && b` inside it branches however the compiler decides.
* On arm64 without LSE, `fetch_add` is a load-exclusive/store-exclusive loop that retries only under contention;
  gcc and clang on aarch64 Linux call libgcc's `__aarch64_ldadd8_relax` instead, which picks LSE or the loop once per
  process. macOS arm64 uses `ldadd` directly.
* On macOS the thread-local access is a call through `_tlv_get_addr`, which may allocate a thread's TLS block on its
  first use. macOS is for development builds only; on Linux the access is one `mov %fs:...`.

### Evidence: machine code

`make -C sdks/c disasm` prints the code of two call sites compiled from `tests/fixtures/callsite.c`:

```c
void site_always(long ok, long a, long b) { HS_ALWAYS(ok != 0, "disasm.always", "cond holds", a, b, 3, 4); }
void site_reach(void) { HS_REACHABLE("disasm.reach", "reached"); }
```

Findings, from the listings below:

* **No syscall instruction** appears in either function (x86-64 `syscall`/`sysenter`/`int 0x80`: count 0; arm64
  `svc`: none). The only calls are `hs__claim` (cold path, first call per thread) and, on macOS, `_tlv_get_addr` and,
  on aarch64 Linux, `__aarch64_ldadd8_relax`. None allocates or locks on the hot path.
* **The only conditional branch is the slot-null test**: `je` on x86-64, `cbz` on arm64. The mask selects compile to
  and/or arithmetic (x86-64) or `csel` (arm64, a conditional select, not a branch).
* **The same instructions execute for a true and a false condition.** `site_always` is 65 instructions on x86-64
  (prologue, body, epilogue), `site_reach` 31.

x86-64, gcc 14.2 `-O2` (Debian container, compile and `objdump` only):

```
0000000000000000 <site_always>:
   0:	push   %rbp
   1:	mov    %rdx,%rbp
   4:	push   %rbx
   5:	mov    %rsi,%rbx
   8:	sub    $0x18,%rsp
   c:	mov    0x0(%rip),%rax        # 13 <site_always+0x13>
  13:	mov    %fs:(%rax),%rdx
  17:	test   %rdx,%rdx
  1a:	je     100 <site_always+0x100>
  20:	mov    (%rdx),%rcx
  23:	add    $0x1,%rcx
  27:	test   %rdi,%rdi
  2a:	mov    %rcx,(%rdx)
  2d:	setne  %dil
  31:	movzbl %dil,%edi
  35:	mov    %rdi,%rsi
  38:	mov    %rdi,%r9
  3b:	xor    $0x1,%rsi
  3f:	lock xadd %r9,0x0(%rip)        # 48 <site_always+0x48>
  48:	mov    %rsi,%r8
  4b:	lock xadd %r8,0x0(%rip)        # 54 <site_always+0x54>
  54:	lea    0x0(%rip),%r10        # 5b <site_always+0x5b>
  5b:	mov    %r9,%rax
  5e:	neg    %rax
  61:	or     %r9,%rax
  64:	shr    $0x3f,%rax
  68:	xor    $0x1,%rax
  6c:	and    %rdi,%rax
  6f:	lea    0x10(%rdx),%rdi
  73:	lea    -0x1(%rax),%r9
  77:	neg    %rax
  7a:	and    %rdi,%r9
  7d:	and    %r10,%rax
  80:	or     %r9,%rax
  83:	mov    0x8(%rdx),%r9
  87:	mov    %r9,(%rax)
  8a:	mov    %rcx,0x8(%rax)
  8e:	mov    %rbx,0x10(%rax)
  92:	mov    %rbp,0x18(%rax)
  96:	movq   $0x3,0x20(%rax)
  9e:	movq   $0x4,0x28(%rax)
  a6:	mov    %r8,%rax
  a9:	neg    %rax
  ac:	or     %r8,%rax
  af:	mov    0x8(%rdx),%rdx
  b3:	shr    $0x3f,%rax
  b7:	xor    $0x1,%rax
  bb:	and    %rsi,%rax
  be:	lea    -0x1(%rax),%rsi
  c2:	neg    %rax
  c5:	and    %rdi,%rsi
  c8:	lea    0x30(%r10),%rdi
  cc:	and    %rdi,%rax
  cf:	or     %rsi,%rax
  d2:	mov    %rdx,(%rax)
  d5:	mov    %rcx,0x8(%rax)
  d9:	mov    %rbx,0x10(%rax)
  dd:	mov    %rbp,0x18(%rax)
  e1:	movq   $0x3,0x20(%rax)
  e9:	movq   $0x4,0x28(%rax)
  f1:	add    $0x18,%rsp
  f5:	pop    %rbx
  f6:	pop    %rbp
  f7:	ret
  f8:	nopl   0x0(%rax,%rax,1)
 100:	mov    %rdi,0x8(%rsp)
 105:	call   10a <site_always+0x10a>
 10a:	mov    0x8(%rsp),%rdi
 10f:	mov    %rax,%rdx
 112:	jmp    20 <site_always+0x20>
 117:	nopw   0x0(%rax,%rax,1)
0000000000000120 <site_reach>:
 120:	sub    $0x8,%rsp
 124:	mov    0x0(%rip),%rax        # 12b <site_reach+0xb>
 12b:	mov    %fs:(%rax),%rax
 12f:	test   %rax,%rax
 132:	je     1b0 <site_reach+0x90>
 134:	mov    (%rax),%rsi
 137:	mov    $0x1,%ecx
 13c:	add    $0x1,%rsi
 140:	mov    %rsi,(%rax)
 143:	lock xadd %rcx,0x0(%rip)        # 14c <site_reach+0x2c>
 14c:	mov    %rcx,%rdx
 14f:	lea    0x10(%rax),%rdi
 153:	mov    0x8(%rax),%rax
 157:	neg    %rdx
 15a:	or     %rcx,%rdx
 15d:	mov    %rdx,%rcx
 160:	sar    $0x3f,%rdx
 164:	shr    $0x3f,%rcx
 168:	and    %rdi,%rdx
 16b:	lea    0x0(%rip),%rdi        # 172 <site_reach+0x52>
 172:	sub    $0x1,%rcx
 176:	and    %rdi,%rcx
 179:	or     %rcx,%rdx
 17c:	mov    %rax,(%rdx)
 17f:	mov    %rsi,0x8(%rdx)
 183:	movq   $0x0,0x10(%rdx)
 18b:	movq   $0x0,0x18(%rdx)
 193:	movq   $0x0,0x20(%rdx)
 19b:	movq   $0x0,0x28(%rdx)
 1a3:	add    $0x8,%rsp
 1a7:	ret
 1a8:	nopl   0x0(%rax,%rax,1)
 1b0:	call   1b5 <site_reach+0x95>
 1b5:	jmp    134 <site_reach+0x14>
```

Relocations of the same object (the only references outside the function: the thread-local pointer, the site
records in `hs_catalog_v1`, and the cold-path call):

```
f: R_X86_64_GOTTPOFF	hs__tls-0x4
			44: R_X86_64_PC32	hs_catalog_v1+0xdc
			50: R_X86_64_PC32	hs_catalog_v1+0xe4
			57: R_X86_64_PC32	hs_catalog_v1+0xec
			106: R_X86_64_PLT32	hs__claim-0x4
			127: R_X86_64_GOTTPOFF	hs__tls-0x4
			148: R_X86_64_PC32	hs_catalog_v1+0x1c
			16e: R_X86_64_PC32	hs_catalog_v1+0x2c
			1b1: R_X86_64_PLT32	hs__claim-0x4
```

arm64 macOS, Apple clang 21 `-O2` (`otool -tvV`):

```
(__TEXT,__text) section
_site_always:
000	stp	x22, x21, [sp, #-0x30]!
004	stp	x20, x19, [sp, #0x10]
008	stp	x29, x30, [sp, #0x20]
00c	add	x29, sp, #0x20
010	mov	x19, x0
014	adrp	x0, 0 ; 0x0
018	ldr	x0, [x0]
01c	ldr	x8, [x0]
020	blr	x8
024	ldr	x0, [x0]
028	cbz	x0, 0xf4
02c	cmp	x19, #0x0
030	cset	w8, ne
034	ldr	x9, [x0]
038	add	x9, x9, #0x1
03c	str	x9, [x0]
040	eor	x10, x8, #0x1
044	adrp	x11, 0 ; 0x0
048	add	x11, x11, #0x0
04c	ldadd	x8, x12, [x11]
050	add	x13, x11, #0x8
054	ldadd	x10, x13, [x13]
058	add	x14, x0, #0x10
05c	and	x8, x8, #0x1
060	cmp	x12, #0x0
064	csel	x8, x8, xzr, eq
068	add	x12, x11, #0x10
06c	neg	x15, x8
070	and	x12, x15, x12
074	cmp	x8, #0x0
078	csel	x8, x14, xzr, eq
07c	orr	x8, x8, x12
080	ldr	x12, [x0, #0x8]
084	str	x12, [x8]
088	str	x9, [x8, #0x8]
08c	str	x1, [x8, #0x10]
090	str	x2, [x8, #0x18]
094	mov	w12, #0x3
098	str	x12, [x8, #0x20]
09c	mov	w15, #0x4
0a0	str	x15, [x8, #0x28]
0a4	and	x8, x10, #0x1
0a8	cmp	x13, #0x0
0ac	csel	x8, x8, xzr, eq
0b0	ldr	x10, [x0, #0x8]
0b4	add	x11, x11, #0x40
0b8	neg	x13, x8
0bc	and	x11, x13, x11
0c0	cmp	x8, #0x0
0c4	csel	x8, x14, xzr, eq
0c8	orr	x8, x8, x11
0cc	str	x10, [x8]
0d0	str	x9, [x8, #0x8]
0d4	str	x1, [x8, #0x10]
0d8	str	x2, [x8, #0x18]
0dc	str	x12, [x8, #0x20]
0e0	str	x15, [x8, #0x28]
0e4	ldp	x29, x30, [sp, #0x20]
0e8	ldp	x20, x19, [sp, #0x10]
0ec	ldp	x22, x21, [sp], #0x30
0f0	ret
0f4	mov	x20, x2
0f8	mov	x21, x1
0fc	bl	0xfc
100	mov	x1, x21
104	mov	x2, x20
108	b	0x2c
_site_reach:
10c	stp	x29, x30, [sp, #-0x10]!
110	mov	x29, sp
114	adrp	x0, 0 ; 0x0
118	ldr	x0, [x0]
11c	ldr	x8, [x0]
120	blr	x8
124	ldr	x0, [x0]
128	cbz	x0, 0x17c
12c	ldr	x8, [x0]
130	add	x8, x8, #0x1
134	str	x8, [x0]
138	adrp	x9, 0 ; 0x0
13c	add	x9, x9, #0x0
140	mov	w10, #0x1
144	ldadd	x10, x10, [x9]
148	ldr	x11, [x0, #0x8]
14c	add	x12, x0, #0x10
150	add	x9, x9, #0x10
154	cmp	x10, #0x0
158	csel	x9, x9, x12, eq
15c	str	x11, [x9]
160	str	x8, [x9, #0x8]
164	str	xzr, [x9, #0x10]
168	str	xzr, [x9, #0x18]
16c	str	xzr, [x9, #0x20]
170	str	xzr, [x9, #0x28]
174	ldp	x29, x30, [sp], #0x10
178	ret
17c	bl	0x17c
180	b	0x12c
```

On aarch64 Linux, gcc 13.3 and clang 18 produce the same structure with `bl __aarch64_ldadd8_relax` in place of
`ldadd` (`make disasm` prints it).

### Evidence: tests

* `tests/test_nosyscall.c` (T1, Linux): a fresh thread installs a seccomp filter that allows only `exit`,
  `exit_group` and `rt_sigreturn`, then runs 10^6 iterations of every kind (true and false alternating, first
  evaluations and the slot claim included), `HS_WORK` and `hs_thread`, and ends with a raw `exit`. Passes on aarch64
  Linux with gcc 13.3 and clang 18 at -O0, -O2, -O3. Adding one `getpid` to the loop makes it fail.
* Outcome-independent instruction and branch counts (plan T2, `perf_event_open`) are **not** measured yet: that needs
  a Linux host with perf access (see "Not done" below). The listings are the static argument.

## Tests

`make -C sdks/c test` (add `CC=gcc` or `CC=clang`, `OPT=-O0|-O2|-O3`): warnings are errors (`-Wall -Wextra -Werror`).

* `tests/test_unit.c`: formatting and fixed widths, counters and first-witness ownership, merged sites, conflicting
  kinds, thread keys, the 256-slot table with 300 threads, the offline catalog twin, an in-process report (one
  `assert` per id, `end.sites`, `end.threads`, every line within 4096 bytes), a **contention stress** (8 threads x
  100,000 evaluations: exact sums, no torn witness), simultaneous first calls getting distinct slots, and `errno`
  unchanged by a failed write.
* `tests/test_audit.c`: the evaluation audit, with a call-site record deliberately placed outside the catalog section
  (the comparison, no false alarm with several threads, the diagnostic and `end.dropped` after an `exit` report and a
  signal report). `make cxx-templates` builds `tests/fixtures/cxx_templates.cc` (function templates, a class template
  member, an inline function; no ordinary-function site) with its harness `cxx_templates_main.cc`, and checks that each
  id is catalogued (clang: required) or announced as missing, with the right count (other compilers).
  `make audit-control` builds the same fixture against a copy of the header whose records go to a section nobody
  reads and checks that all 7 evaluations are announced; it is part of `make test`. `make cxx-mixed` is the compiler-specific check described
  above; it is not part of `make test`.
* `tests/test_proc.c` with the programs in `tests/fixtures/hs_sample.c` (one binary per scenario, because a binary's
  catalog holds every assertion compiled into it): report shape, the same id at two call sites, fork tags and fresh
  child counters, 100 and 300 threads, crash with and without `HAIRSPRING_SIGNAL_REPORT` (also a null dereference,
  `abort()` and a fault in a worker thread), `hs_exit` and plain-return status, the events file (and a symlink, and an
  unwritable path), `HAIRSPRING_EVENTS=off`, `eval` lines, stderr closed with and without descriptor reuse, a timer
  signal during the report, fork while another thread is inside `hs_thread`, a dup2 onto the events descriptor (also before an `eval` line, and in the low-limit fallback), an inherited ignored SIGTERM with handlers requested, a descriptor limit too low for a private copy, an empty catalog, constant thread-line length, and **two processes writing to one pipe**
  (Linux) or one events file (macOS), 20 rounds, every line checked to be one whole object.
* AddressSanitizer and UBSan: `make -C sdks/c test BUILD=build/asan EXTRA="-fsanitize=address,undefined"` is clean on
  macOS. Two tests skip with a message under ASan, because ASan owns SIGSEGV (and SIGBUS, and on macOS also SIGFPE) and the SDK leaves them alone: the
  SIGSEGV crash tests, with and without `HAIRSPRING_SIGNAL_REPORT` (the SIGABRT test still runs).
* `make nosyscall` and `make nosyscall-pic` (T1, Linux): the seccomp test, built normally and with `-fPIC`.
* `make staticfail`: an id of 129 bytes, a message of 513 bytes and five values are rejected at compile time.
* `make cxx`: the macros compile and report from C++ (skipped when the matching C++ compiler is absent).
* `make -C sdks/c conformance` or `cargo test -p hairspring-props --test c_sdk` (23 tests): the same programs' output is
  folded by the reference parser. Checks: no degradation, an always-false is a `Violation` with its witness, a
  never-hit `reachable` is `Unmet`, merged sites sum, fork tags, **fork then exec of an instrumented and of an
  uninstrumented image**, a spawned child's tag and `tag_stable`, an exec without fork (`<tag>.x1`), thread counts (101 and 256), threads still running at exit,
  `report_missing` after a crash with handlers off, a worker-thread fault with handlers on, `eval` marks `perturbing`,
  ids and messages at their limits, a catalog conflict, an empty catalog, more ids than the limit (degraded), and two
  processes on one output.
  `cargo run -p hairspring-props --example fold_events -- file.events` prints the folded verdicts of any capture.

### Pipe atomicity differs by platform

A write of at most `PIPE_BUF` bytes to a pipe is atomic. `PIPE_BUF` is 4096 on Linux, so a report line (at most 4096
bytes) never interleaves with another process's. It is **512 on macOS**, and a test run showed lines of about 600
bytes (an `assert` with two witnesses) being split when two processes shared a pipe. Lines are not shortened for it:
macOS is for development, and its two-process tests share an `O_APPEND` events file (`HAIRSPRING_EVENTS_FILE`)
instead of a pipe. Use the events file for multi-process runs on macOS.

## Not done

* Rust SDK, `hs_choice` and the tape (layout not agreed), `hairspring catalog <binary>` (C2), shared objects.
* Plan T2 (perf counters) and T3 (schedule perturbation under Hermit): need the Linux runner with perf and Hermit.
* Verification of the C++ template behaviour under g++ on Linux after this change (the detection is tested with clang
  on macOS, with the section deliberately bypassed; the g++ run is pending).
* The full test run on real x86-64 Linux. x86-64 was compiled and disassembled only (an emulated container); the tests
  ran natively on aarch64 Linux.
* The engine treating `end.dropped > 0` as degradation (a follow-up in `hairspring-props`).
