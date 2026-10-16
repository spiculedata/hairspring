# Guest event protocol v1

How an instrumented program tells Hairspring about its assertions. The machine-readable contract is
[`sdks/conformance/schema/v1.json`](../sdks/conformance/schema/v1.json); the reference parser is the
`hairspring-props` crate. This document is the prose companion. Protocol v1 is frozen: later changes are additive, or
a v2.

## Status

M1 partial: the protocol and the parser are done; the C SDK (C1) is outstanding. The wire format is frozen once the
M1 review findings are closed.

## Transport

A line on stdout or stderr that starts with `HAIRSPRING_EVENT ` (one space) followed by one JSON object. Every other
line is ordinary program output and is ignored (a line such as `HAIRSPRING_EVENTS_FILE=...` is ordinary output).

Lines are limited to 4096 bytes including the prefix, must be valid UTF-8, and may not contain duplicate object keys or
nesting deeper than 4. There is one line cap everywhere. SDKs MUST write each line with a single `write(2)` of at most
4096 bytes, so that concurrent writers to a pipe cannot interleave inside a line (`PIPE_BUF` atomicity).

Counters may be right-aligned in a 20-character field (JSON allows the padding). SDKs do this so a report's length does
not depend on the counter values, which keeps syscall arguments identical across outcomes.

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
| `hello` | first line per process | `proc`, `sdk`, `sdk_version`, `catalog_hash`, `signal_handlers` |
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
  interval and the rule of three (3/N, when the count is zero) are computed over these only, so a trial that died before
  reporting is not mistaken for a trial in which nothing went wrong. `trials_unreported`, `trials_degraded` and
  `trials_perturbing` say how many trials were excluded or flagged.
* A property with no rated trial has **no rate**: `PropertyStat.rate` is `None`, `unrated_reason` is
  `NoRatedTrials`, and `rate_text()` reads `unrated (0 rated trials)`. This is not a rate of 0%. Degraded trials are
  never rated, though their failures still count as evidence.
* The rate is of trials false (always-type), trials true (`sometimes`) or trials hit (`reachable`, `unreachable`).
* `first_failure` is the lowest failing seed, whatever order trials were added in.
* If a trial declares an id with a different kind from the first trial that did, its data for that id is not merged;
  the mismatch and that trial's own failure are listed in the run summary.

## Versioning and unknowns

`v` is a major version. A line with `v` greater than 1 is counted and ignored. Within v1, unknown kinds and unknown
fields are counted and ignored, so new fields and kinds can be added without breaking old engines.

## Limits and degradation

Ids and process tags are 1 to 128 and 1 to 64 bytes of `[A-Za-z0-9._:/-]`; thread keys are 1 to 64 bytes of the same
set plus `#`; messages are at most 512 bytes. At most 4096 ids per process, 256 `thread` lines per process, 64
processes and 1 MiB of event lines per trial. Malformed, over-limit, duplicate and uncatalogued lines are dropped
(duplicate asserts are summed) and counted; a trial with any of those, a conflict or a failed cross-check is marked
degraded.

A process with no `end` line is reported as `report_missing`: its report was lost or cut, or it crashed with signal
handlers off.

## Tape v1

Stub. Only the layout is still open: how a thread's stream is carved from the tape file. It is to be agreed with W2
before `hs_choice` ships at M2. The unit of `tape_consumed` is already fixed: bytes (see Messages).

## Legacy

`HAIRSPRING_VIOLATION:` lines and a non-zero exit status remain violations, independent of this protocol.
