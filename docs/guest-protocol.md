# Guest event protocol v1

How an instrumented program tells Hairspring about its assertions. The machine-readable contract is
[`sdks/conformance/schema/v1.json`](../sdks/conformance/schema/v1.json); the reference parser is the
`hairspring-props` crate. This document is the prose companion. Protocol v1 is frozen: later changes are additive, or
a v2.

## Status

Status: the protocol and the parser are done; the C SDK is in [`sdks/c`](../sdks/c) (see [c-sdk.md](c-sdk.md)), the Go
SDK in [`sdks/go`](../sdks/go), the Rust SDK in [`sdks/rust`](../sdks/rust) (see [rust-sdk.md](rust-sdk.md)) and the
Python SDK in [`sdks/python`](../sdks/python) (see [python-sdk.md](python-sdk.md)) and the Java SDK in
[`sdks/java`](../sdks/java) (see [java-sdk.md](java-sdk.md)). The wire format is frozen once the review findings are
closed.

## Transport

A line on stdout or stderr that starts with `HAIRSPRING_EVENT ` (one space) followed by one JSON object. Every other
line is ordinary program output and is ignored (a line such as `HAIRSPRING_EVENTS_FILE=...` is ordinary output).

Lines are limited to 4096 bytes including the prefix, must be valid UTF-8, and may not contain duplicate object keys or
nesting deeper than 4. There is one line cap everywhere. SDKs MUST write each line with a single `write(2)` of at most
4096 bytes, so that concurrent writers to a pipe cannot interleave inside a line (`PIPE_BUF` atomicity).

Counters may be right-aligned in a 20-character field (JSON allows the padding). SDKs do this so a report's length does
not depend on the counter values, which keeps syscall arguments identical across outcomes. The same whitespace rule
lets an SDK pad a `first_false` / `first_true` value (object or `null`) to a constant width.

### Leniency

The parser accepts extra spaces between the prefix and the object (`HAIRSPRING_EVENT   {...}`) and a trailing `\r`.
It rejects, where an integer is expected, `-0`, `1e2`, `1.0`, `01` and `+1`; negative numbers where unsigned ones are
required; and integers outside `u64` (or `i64` for `vals` and `status`).

### Strings

Ids, process tags and thread keys are ASCII (see Limits), so their byte and character counts are equal. Free-text
strings (`msg`, `sdk`, `sdk_version`, `catalog_hash`, `loc.file`, `loc.func`) are limited in **bytes**, not characters;
the schema states this with `x-max-bytes` because `maxLength` counts code points. They may not contain control
characters or Unicode format and separator characters (Cf, Zl, Zp: for example U+200B, U+202E, U+2028, U+FEFF).

## Messages

Every object carries `"v": 1` and a `"kind"`.

| kind | when | fields |
|---|---|---|
| `hello` | first line per process | `proc`, `sdk`, `sdk_version`, `catalog_hash`, `signal_handlers`, optional `tag_stable` |
| `catalog` | once per assertion id, at startup | `proc`, `id`, `type`, `must_hit`, optional `msg`, `loc` |
| `assert` | exactly once per catalogued id, at exit | `proc`, `id`, `hits`, `trues`, `falses`, optional `first_false`, `first_true` |
| `work` | at exit | `proc`, `ops` |
| `thread` | once per thread, at exit | `proc`, `thread`, `evals`, `tape_consumed`, optional `key_stable` |
| `end` | last line per process | `proc`, `reason` (`exit`, `signal`, `explicit`), `status`, `sites`, `evals`, `dropped`, `threads` (a count) |
| `eval` | per evaluation, scripts only | `id`, `type`, `ok`, optional `proc` (default `_eval`) |

`first_false` and `first_true` are `{thread, ord, vals}`: the thread key, that thread's own ordinal (from 1), and
exactly four integers. Ordinals are per thread because a global counter would depend on the interleaving.

`thread` has its own line, one per thread, so a process with up to 256 threads never outgrows the line cap. `end.threads`
is the number of `thread` lines the process sent, and `end.sites` the number of catalogued ids. If `end.threads`
disagrees with the lines that arrived, or `end.sites` with the catalog, or a catalogued id has no `assert` although
`end` arrived, the trial is marked degraded. `key_stable` defaults to true and is false for keys assigned by claim
order (`t#1`), which may differ between schedules.

`end.status` is advisory: it is what the SDK could learn about the exit status (a signal number for `reason:
signal`, the value passed to `exit` where the runtime shows it, otherwise 0). The engine always uses the operating
system's wait status to decide how a process ended.

`tape_consumed` is the number of bytes read from that thread's tape stream, counting reads past the end of the tape
(an exhausted tape returns zeros and keeps counting).

A trial that contains any `eval` line is marked `perturbing`: each such line is a write syscall during the run, so the
schedule differs from an uninstrumented one. Perturbing trials are counted separately and excluded from tier claims.
`ok` is ignored on `eval` lines for `reachable` and `unreachable`, which have no condition. An `eval` line always
belongs to the reserved tag `_eval`: its `proc` field may be omitted or be `_eval`, and any other value (a real
process's tag) is rejected, so `eval` can never merge into an SDK process. `eval` lines never create a process entry,
so an eval-only trial does not look like a lost report.

### Process tags

Every process in a trial MUST have a unique `proc` tag. An SDK's fork handler derives the child's tag from its
parent's deterministically (`<parent>.f<n>`, `n` counting the parent's forks). The tag `_eval` is reserved for `eval`
lines. Duplicate `hello`, `work` and `end` lines for one tag, and duplicate `thread` lines for one thread key, keep
the first and are counted as duplicates.

`hello.tag_stable` (optional, default true) is false when the tag was derived from a pid rather than from the process
tree (the C SDK's `<tag>.s<pid>` for a child started with `posix_spawn` or `system()`, and everything below it). Such a
tag can differ between runs of one schedule, like an unstable thread key, so it must not be used for cross-run
comparison. It is additive: a parser that does not know the field ignores and counts it.

A backend MUST strip any `HAIRSPRING_PROC` and `HAIRSPRING_PROC_PID` from the environment it gives a guest, or set
`HAIRSPRING_PROC` itself and no `HAIRSPRING_PROC_PID`; a stray inherited pair makes the first process look like a
spawned child.

SDKs MUST send exactly one `assert` per `(proc, id)`, merging call sites that were inlined or duplicated (the same id
at two call sites is one catalogued id: `catalog` may be repeated, and `end.sites` counts distinct ids after merging). If a second
`assert` for the same pair does arrive, its counts are **added** (nothing that could be a failure is discarded) and
the trial is marked degraded.

### Order

Within a process, `catalog` is declared before `assert` in stream order, but the engine does not depend on it: it
applies `hello`, `catalog`, `thread`, `work` and `end` as they arrive and applies `assert` and `eval` after the stream
ends, so an `assert` that precedes its `catalog` line is still counted. If the same id is declared with different
kinds, the kind declared by the process with the lowest `proc` tag is kept (so the outcome does not depend on which
process spoke first); within one process the first declaration in stream order is kept.

## Assertion kinds

| kind | trial-level failure (a finding) | run-level UNMET (coverage gap) |
|---|---|---|
| `always` | any evaluation false | never evaluated in any trial |
| `always_or_unreachable` | any evaluation false | never |
| `sometimes` | never | never true in any trial |
| `reachable` | never | never hit in any trial |
| `unreachable` | hit at least once | never |

`must_hit` is sent explicitly and must agree with the kind: true for `always`, `sometimes` and `reachable`.
`hits` must equal `trues + falses`.

`reachable` and `unreachable` have no condition. SDKs MUST send `trues = hits`, `falses = 0` and only `first_true`
for them; engines use only `hits`.

## Kind conflicts are trial errors

The same id declared with two different kinds, within one process or across processes, is a catalog conflict. It is a
**trial-level error**: `TrialProps::verdict()` reports it as a `CatalogConflict` failure, the trial is degraded, and
the run-level status of the id is `Violation`, never `Unmet`. Counts from the conflicting declaration are not merged
into the kept one; any `assert` line dropped for that reason is counted in `dropped_asserts`, and its counts (hits, trues, falses) are
attached to the conflict failure for diagnosis.

## Run-level statistics

Per id, evidence counts and rate counts are kept separately.

* `trials`: trials that catalogued the id. `trials_hit`, `trials_true`, `trials_false`, `trials_conflicted` come from
  all of them, because a failure seen in a crashed or degraded trial is still a failure (and decides `Violation`).
* `trials_rated`: trials in which every process sent `end` and no event data was dropped or contradictory. The Wilson 95%
  interval and the exact zero-event bound (1 - 0.05^(1/N), when the count is zero; about 3/N for large N) are computed over these only, so a trial that died before
  reporting is not mistaken for a trial in which nothing went wrong. `trials_unreported`, `trials_degraded` and
  `trials_perturbing` say how many trials were excluded or flagged.
* A property with no rated trial has **no rate**: `PropertyStat.rate` is `None`, `unrated_reason` is
  `NoRatedTrials`, and `rate_text()` reads `unrated (0 rated trials)`. This is not a rate of 0%. Degraded trials are
  never rated, though their failures still count as evidence.
* The rate is of trials false (always-type), trials true (`sometimes`) or trials hit (`reachable`, `unreachable`).
* `first_failure` is the lowest failing seed, whatever order trials were added in.
* If a trial declares an id with a different kind from the first trial that did, its data for that id is not merged;
  the mismatch and that trial's own failure are listed in the run summary.

## How the Hermit backend judges a trial

In this order, the first rule that applies decides:

1. Hermit's own failure, or a misconfigured events setup (a Config fault): the run is an error and no trial is spent.
2. A timeout: inconclusive (or a violation when the target sets `hang_is_violation`).
3. The host failed to collect the events (an Infra fault: scratch I/O or cleanup failed): inconclusive, never a pass.
4. A trial-level property failure (`always` false, `unreachable` hit, catalog conflict): the trial fails and gets a
   bundle recording `property` and `property_kind`. This holds for degraded trials too: a false that was read is a
   false.
5. A `HAIRSPRING_VIOLATION:` line.
6. A line on stderr that begins `Deadlock detected: ` (Hermit's report that every thread is blocked), together with a
   non-zero exit: a deadlock finding. It is reported as a deadlock with Hermit's own line, the bundle records
   `failure_kind=deadlock`, and `hang_is_violation` does not change it (Hermit detected it; no timeout was involved).
   Confirmation reruns and replay must fail the same way (a deadlock, not an ordinary violation) to count.
7. Otherwise the exit status, as before.
8. Otherwise a clean exit is a pass, unless the events are unusable (a Guest fault such as an unreadable, oversized or
   truncated file or an output stream that never closed; lost or contradictory data; a process with no `end`). Such a
   trial is degraded and inconclusive: it is never a pass, is not rated, and makes no must-hit claim.

UNMET (`sometimes`, `reachable`, `always` never evaluated) is a run-level finding over claimable trials only (rated and
not perturbing). It never fails a trial, and it is listed in its own section of the report. A run that stops at its first
failure, or tested nothing, makes no UNMET claim: such a property is `Unrated` in the data. `confirm` reruns count only
when they fail the same property ids, kinds and reasons and reproduce the behaviour hash (which includes the reported
events); reruns that could not be judged (a timeout, a host or events fault) are counted as inconclusive, not as
non-reproduction. Replay of a bundle with a `property` also requires that property to fail again.

A trial that timed out is inconclusive: the report of a killed process decides nothing. A failure it shows is not a
finding and is not recorded in the bundle; the run counts it in `trials_inconclusive_with_failures` and the report says
"seen in an inconclusive trial".

## The run-level rate and claim text

The report of a run (`hairspring test`, `hairspring show`) states how many trials hit, out of how many were rated.
One job runs many trials, and a hit is a trial in which a property failed. The figures and the wording are computed
from counts and recorded labels by two pure modules in `hairspring-props`, `run_rate` and `run_claim`, so another
front end can print the same lines.

* A rate is always `k/n rated trials` with its Wilson 95% interval. A bare percentage is never printed.
* The denominator is the rated trials. Inconclusive trials (timeouts and unusable events) and crashed trials are
  listed on a `not rated trials:` line, are never in the denominator and are never described as passes. The CLI reads
  only the inconclusive count from a stored outcome; the crashed count is zero there, because a host or Hermit failure
  ends the run as an error. Starved and infrastructure trials do not have columns of their own yet.
* If every non-rated trial had been a hit, the rate would be `(k + x)/(n + x)`. The report prints that as a
  `sensitivity:` line with its own interval. It is left out when nothing was rated, because it would only say that
  all of them hit.
* With nothing rated the report says `0/0 rated trials` and that there is no rate. It never says 0%. A run in which
  every trial was inconclusive ends as an error and still shows this.
* Zero hits print one bound: the exact one-sided 95% upper bound, with the rule of three (about 3/N) as a note from 30
  rated trials, where it is close. Below that the exact bound alone is shown, because 3/N overshoots. The two-sided
  Wilson interval is shown only when there is at least one hit.
* Upper bounds are rounded up and lower bounds down; a figure below 0.01% is written `<0.01%`.
* A run that stopped at its first hit prints its counts and "not a rate": the stop decided how many trials it ran.
  A failed run is treated this way even if an older stored record lacks the stopped flag.
* If a stored inconclusive count is larger than the trial count, it is capped and a `note:` line says so.
* Not done: failures per core-hour with a Garwood interval, the exact rate-ratio test for comparing two builds, pooling
  within an arm, and the "uncalibrated" label for a zero from an arm with no positive control.
* The word is "hits". The report does not say "found", "discovered", "unassisted" or "bug rate".

Claim text follows the same rule: it is generated from the recorded label (build record, declared knowledge flags,
seed selection) and the counts, never from free text. `chips:` shows the tier, seed selection and every declared flag
beside the claim. When trials were left out of the count, the sentence says how many were not rated. A label with no
declared knowledge list makes no claim and says so, a hit with no confirming replay is a candidate with the
`single_run` caveat and no sentence, and the strongest phrase is "unmodified binary, blind seeds"
followed by the configuration note. The word "unassisted" is never generated, because it cannot be derived from the
recorded fields. A declared `window_widening` forces the `assisted` tier.

## Versioning and unknowns

`v` is a major version. A line with `v` greater than 1 is counted and ignored. Within v1, unknown kinds and unknown
fields are counted and ignored, so new fields and kinds can be added without breaking old engines.

## Limits and degradation

Ids and process tags are 1 to 128 and 1 to 64 bytes of `[A-Za-z0-9._:/-]`; thread keys are 1 to 64 bytes of the same
set plus `#`; messages are at most 512 bytes. At most 4096 ids per process, 256 `thread` lines per process, 64
processes and 4 MiB of event lines per trial. Malformed, over-limit, duplicate and uncatalogued lines are dropped
(duplicate asserts are summed) and counted; a trial with any of those, a conflict or a failed cross-check is marked
degraded.

The 4 MiB event budget (`max_event_bytes_per_trial` in the schema; 1 MiB before it was raised) applies to each
transport separately (stdout, stderr and the events file), and within one transport it is shared by every process in the
trial, newlines included. The fold then applies the budget again to the merged stream, so the events kept per trial
never exceed it, but a trial that fills all three transports can hold up to about 3x the budget in memory while it runs.
A catalog line plus an assert line cost about 850 bytes per id, so the budget holds about 4,900 ids, or about 1,200 ids per MiB summed over
processes. Backends take the budget as a setting (`HermitSettings::max_event_bytes`, default 4 MiB). The capture buffer
is a cap, not an allocation: it grows as events arrive, so only trials that emit events spend memory.

A process with no `end` line is reported as `report_missing`: its report was lost or cut, or it crashed with signal
handlers off.

## Tape v1

Stub. Only the layout is still open: how a thread's stream is carved from the tape file. It is to be agreed with the
search component before `hs_choice` ships. The unit of `tape_consumed` is already fixed: bytes (see Messages).

## Legacy

`HAIRSPRING_VIOLATION:` lines and a non-zero exit status remain violations, independent of this protocol.
