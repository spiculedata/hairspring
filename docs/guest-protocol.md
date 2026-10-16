# Guest event protocol v1

How an instrumented program tells Hairspring about its assertions. The machine-readable contract is
[`sdks/conformance/schema/v1.json`](../sdks/conformance/schema/v1.json); the reference parser is the
`hairspring-props` crate. This document is the prose companion. Protocol v1 is frozen: later changes are additive, or
a v2.

## Transport

A line on stdout or stderr that starts with `HAIRSPRING_EVENT ` (one space) followed by one JSON object. Every other
line is ordinary program output and is ignored. Lines are limited to 4096 bytes including the prefix, must be valid
UTF-8, and may not contain duplicate object keys, control characters inside strings, or nesting deeper than 4.

Counters may be right-aligned in a 20-character field (JSON allows the padding). SDKs do this so a report's length does
not depend on the counter values, which keeps syscall arguments identical across outcomes.

## Messages

Every object carries `"v": 1` and a `"kind"`.

| kind | when | fields |
|---|---|---|
| `hello` | first line per process | `proc`, `sdk`, `sdk_version`, `catalog_hash`, `signal_handlers` |
| `catalog` | once per assertion id, at startup | `proc`, `id`, `type`, `must_hit`, optional `msg`, `loc` |
| `assert` | once per catalogued id, at exit | `proc`, `id`, `hits`, `trues`, `falses`, optional `first_false`, `first_true` |
| `work` | at exit | `proc`, `ops` |
| `end` | last line per process | `proc`, `reason` (`exit`, `signal`, `explicit`), `status`, `sites`, `evals`, `dropped`, `threads` |
| `eval` | per evaluation, scripts only | `id`, `type`, `ok`, optional `proc` (default `p0`) |

`first_false` and `first_true` are `{thread, ord, vals}`: the thread key, that thread's own ordinal, and exactly four
integers. Ordinals are per thread because a global counter would depend on the interleaving. `threads` entries are
`{thread, evals, tape_consumed, key_stable}`; `key_stable` defaults to true and is false for keys assigned by claim
order (`t#1`), which may differ between schedules.

A trial that contains any `eval` line is marked `perturbing`: each such line is a write syscall during the run, so the
schedule differs from an uninstrumented one. Perturbing trials are excluded from tier claims.

## Assertion kinds

| kind | trial-level failure (a finding) | run-level UNMET (coverage gap) |
|---|---|---|
| `always` | any evaluation false | never evaluated in any trial |
| `always_or_unreachable` | any evaluation false | never |
| `sometimes` | never | never true in any trial |
| `reachable` | never | never hit in any trial |
| `unreachable` | hit at least once | never |

`must_hit` is sent explicitly and must agree with the kind: true for `always`, `sometimes` and `reachable`.
For `reachable` and `unreachable`, only `hits` is significant. `hits` must equal `trues + falses`.

Statistics per id: trials hit, true and false; a Wilson 95% interval on trials false (always-type), trials true
(`sometimes`) or trials hit (`reachable`, `unreachable`); and the rule of three (3/N) when the count is zero.

## Versioning and unknowns

`v` is a major version. A line with `v` greater than 1 is counted and ignored. Within v1, unknown kinds and unknown
fields are counted and ignored, so new fields and kinds can be added without breaking old engines.

## Limits and degradation

Ids and process tags are 1 to 128 and 1 to 64 bytes of `[A-Za-z0-9._:/-]`; thread keys are 1 to 64 bytes of the same
set plus `#`; messages are at most 512 bytes. At most 4096 ids per process, 256 thread entries per `end`, 64 processes
and 1 MiB of event lines per trial. Malformed, over-limit, duplicate and uncatalogued lines are dropped and counted; a
trial with any of those is marked degraded. The same id declared with two kinds is a catalog conflict: the first kind is
kept and the conflict is reported.

A process with no `end` line is reported as `report_missing`: its report was lost or cut, or it crashed with signal
handlers off.

## Legacy

`HAIRSPRING_VIOLATION:` lines and a non-zero exit status remain violations, independent of this protocol.
