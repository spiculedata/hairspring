# Showcase plan: reproduce the SQLite WAL-reset bug

**Status: planned, not done.** This documents a feasibility assessment and a concrete plan. Nothing here has
been run yet; the claims below are from SQLite's published documentation.

## The bug

From SQLite's WAL documentation: a data race that can corrupt a database or lose committed writes.

- Affected: SQLite 3.7.0 (2010) through 3.51.2 (2026-01-09). Fixed in 3.51.3; backports exist for 3.44.6 and 3.50.7.
- Needs: WAL mode, **two or more connections** to the same database (threads or processes), and tight timing.
- Sequence (as documented): a checkpoint completes; a second checkpoint starts shortly after; while it starts,
  another connection commits a transaction that resets the WAL; due to a data race the second checkpoint does not
  notice the reset and leaves a wrong field in the WAL-index header; later transactions grow the WAL past the
  first checkpoint's threshold; a third checkpoint then skips part of a transaction, so committed data never
  reaches the database file.
- Antithesis reported finding it in about 15 minutes with a generic workload (concurrent writers and checkpoints
  plus integrity checks).

## Why it is a good fit

It is exactly the problem Anthill's Linux backend exists for: a rare interleaving of real, unmodified C code that
needs controlled scheduling, found by a generic workload plus a strong property. On the Ubuntu 24.04 machine we used, Python's SQLite library reports 3.45.1, inside the affected
range. A distribution may have backported the fix without changing the version string (check
`apt changelog libsqlite3-0`), so the plan below builds 3.51.2 and 3.51.3 from source for a reliable comparison.

## Plan

1. **Workload** (a small C program, run under Hermit): N writer connections doing short write transactions with
   unique, checkable rows, and one or two connections issuing `PRAGMA wal_checkpoint(...)` in a loop, all on one
   WAL-mode database file (in a directory outside `/tmp` and the home directory, which Hermit replaces/restricts).
2. **Property**: after the workload, reopen the database; every committed transaction's rows must be present and
   `PRAGMA integrity_check` must return `ok`. Print `ANTHILL_VIOLATION: <what is missing>` otherwise.
3. **Targets**: `sqlite-3.51.2` (affected) and `sqlite-3.51.3` (fixed), same workload, built from the amalgamation.
4. **Exploration**: `kind = "process"` under Hermit with random scheduling (and `--chaos`), 100s of seeds.
5. **Success**: a failing seed on 3.51.2 that reproduces 3/3, and the same number of seeds passing on 3.51.3.

## Risks (why this may not work first time)

- **The race window is tiny.** Hermit preempts guest threads by counting retired branches; whether it can land a
  preemption inside the specific window depends on its timeslice granularity and scheduler heuristics. This is
  the main uncertainty and the reason the first step is a feasibility spike, not the full harness.
- **Threads versus processes.** The documented bug applies to both; the WAL index lives in a shared-memory file
  mapped by every connection. How Hermit schedules multiple processes sharing an `mmap`ed file is untested.
- **Speed.** Hermit runs are roughly an order of magnitude slower than native, which limits seeds per hour.
- **File locking** (`fcntl`) and `mmap` semantics under Hermit's sandbox are unverified for this workload.

## First spike (about an afternoon)

On the Linux bare-metal box: build the workload with the system SQLite, run 50 seeds under Hermit with
`--sched-heuristic=random`, and see whether (a) it runs at all, (b) results vary by seed, (c) any seed violates
the property. If (a) and (b) hold, scale up; if not, adjust the preemption settings before investing further.
