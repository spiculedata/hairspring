# SQLite WAL-reset experiment

An attempt to reproduce SQLite's WAL-reset data race (present in 3.7.0 to 3.51.2, fixed in 3.51.3; see
<https://sqlite.org/wal.html>, section 11) under Hairspring's Hermit backend. Motivated by the public write-up at
<https://antithesis.com/blog/2026/wal-reset-bug/>. Everything here is an experiment log, not a product feature.

## Status

**Assisted reproduction achieved; unassisted finding not achieved.**

- Under Hermit (`--chaos --max-timeslice=1000000`, sched-seed 3) a targeted workload (`src/walwork8.c`, mode 4, group 3;
  `walwork9.c` with `yields=1` is equivalent) on SQLite 3.51.2 built with a `nanosleep` inserted at the WAL checkpoint's
  header check loses exactly one committed transaction (`committed=300 counter=299 rows=299`) in **6 of 6** runs: 4 at the
  default skid margin and 2 at `--skid-margin=25`, with identical counts (7903 syscalls, 4583 turns, 124 slices) in all six.
  The same flags on 3.51.3 give **0 of 29** violations, including seed 3 twice. Across the 28-seed arm, 1 of 28 seeds hits.
  The sleep's length did not matter in the range that hit (see "Method notes"); its effect is inferred to be turning the
  check into a scheduling point.
- Natively, the same jittered patch (delay 60 to 180 microseconds) hits 186 of 1641 runs on 3.51.2 and 0 of 2363 on
  3.51.3 (`logs/native_mode4_jdelay.log`; that script prints `delay_us=0` because it reports a different variable than the
  `HS_DELAY_BASE_US=60 HS_DELAY_SPREAD_US=120` pair that was actually set).
- `--verify` passes at skid margins 100 (default), 50 and 25 on the unmodified build, and the schedule is identical across
  margins for the same seed.
- On the unmodified 3.51.2 build, mode 4: **0 violations in 11,611 dangerous-window executions across 56 distinct
  schedules** (runs that finished normally, from the `cycles` field in `logs/pilot*.results`; a distinct schedule is a
  distinct tuple of workload version, arguments, Hermit flags without skid/verify options, and seed). The 95% upper bound on
  the per-window rate is 3/N, about 2.6e-4. That bound is uninformative: the delay-assisted control shows the achievable rate
  is far lower.
- With two writer yields (`yields=2`) the checkpointer is resumed at a scheduler-determined point about five syscalls into
  the writer's next transaction, where the stale checkpoint is harmless; with one yield it resumes at the park. Hits were 0 of 7
  on 3.51.2 with two yields.

## Not claimed

Finding the bug without the patch; random exploration finding it; a generic workload; any rate comparison with other tools.
The workload is single-process; SQLite's cross-process WAL protocol was not exercised. An earlier statement that the
unassisted gap is a "scheduler policy limit" was withdrawn: the evidence only shows where the checkpointer resumes under two
specific workloads.

## Environment

Intel Xeon E5-1410 v2 (8 logical CPUs), Ubuntu 24.04, kernel 6.8.0-146, `perf_event_paranoid=1`, gcc 13.3.0 (`-O1`),
Hermit: <https://github.com/rrnewton/hermit> at commit `25781c5c48d47c316324fd912b7dcafa394c339a`, all runs with
`--epoch=2026-01-01T00:00:00Z`, SQLite amalgamations 3.51.2 and 3.51.3 (SHA-256 in `setup.sh`).

## Layout

- `setup.sh` downloads and verifies the two amalgamations and builds the baseline workload.
- `src/walwork*.c` workload versions; see the table. The property is: no committed write is lost and
  `PRAGMA integrity_check` is ok; failures print `HAIRSPRING_VIOLATION:`. Argument order is in each usage line.
- `patches/` scripts that patch the amalgamation: `patch_yield.py` (scheduling points), `patch_delay.py` (fixed delay in
  `walCheckpoint`), `patch_jdelay.py` (deterministic jittered delay, taken only on a fully backfilled WAL).
- `scripts/` the runners and monitors that produced the logs. Set `BOX=user@host` for the monitor.
- `logs/` raw result files, one line per run.

| Source | Use |
|---|---|
| `walwork.c` | v1: insert-heavy writers, free-running checkpointer (`spike*`, `sweep.results`) |
| `walwork2.c` | v2: counter plus log row; free-running checkpointer (`sweep2`, `sweep3`) |
| `walwork4.c` | v3: **null by construction** (the dangerous state never arises); do not reuse |
| `walwork5.c`, `walwork6.c` | v4, mode 3: **superseded**; `sweep5` control was invalid, `sweep6` used mode 3 |
| `walwork7.c`, `walwork8.c` | mode 4 (strict alternation); 8 adds removal of the database files at exit (needed for `--verify`) |
| `walwork9.c` | adds the `yields` argument; final |

Mode 4: the writer commits `group` transactions then parks; the checkpointer backfills fully, releases the writer, and
immediately runs the racing checkpoint.

## Which log is which

| Log | Workload | What it shows |
|---|---|---|
| `spike.log` | v1, `random` heuristic | the first runs hung (livelocked checkpointer, 5 timeouts) |
| `spike2.log` | v1 (bounded checkpointer) | 100 seeds per version, 0 violations |
| `sweep.results` | v1 | 240 plain runs (1 timeout) and 300 runs with scheduling points, 0 violations |
| `sweep2.results` | v2, free-running checkpointer, delay hints | 0 violations of 234 rows, 20 timeouts (inconclusive); estimated about 2 PASSIVE checkpoints per run |
| `sweep3.results` | v2 lockstep, 5 ms delay | 7 violations in 300 runs; the sleep held the checkpoint lock, which blocked the writer's autocheckpoints and let WAL generations grow, so it is not a pure window widener |
| `sweep4.results`, `sweep5.results` | v3, v4 | invalid designs; the control hit 0 of 30 |
| `sweep6.results` | mode 3 (gated) | control 14 of 20 on 3.51.2, 0 of 20 on 3.51.3; unassisted 0 of 80 completed runs; the 0.25 ms leg timed out (12 of 13) |
| `pilot*.results` | mode 4 | cost pilots, skid margins, `--verify`, the jittered control, double-yield (`pilot7`) |
| `native_mode4*.log` | mode 4 native | native controls (see Status) |

Timeouts (`rc=124`) are inconclusive, never passes.

## Method notes

- Seeds only change the schedule under `--sched-heuristic=random`, `stickyrandom` or `--chaos`. A passing run always prints the
  same line, so output equality says nothing about schedule diversity; compare `--summary-json` counts instead.
- Native numbers from the earliest exploratory workloads were not logged to disk and are not quoted.
- Control delays: 15 ms (0 of 7) and an 8 to 20 ms jittered delay (0 of 7) gave no hits, and a 550 to 900 microsecond arm gave
  0 of 28. Only the 100 to 400 microsecond arm produced a hit; within it the outcome class of a sleep did not depend on the
  value (in a logged run: 28 zero / 61 full / 62 parked / 10 partial writer-progress outcomes across 161 sleeps). The reading
  that a writer transaction takes well under a millisecond of virtual time, so the writer finishes its group during a long
  sleep, is inferred from the extra syscalls per cycle, not directly measured.
- Two logged diagnostic runs (seed 3, yields 1 and 2) were classified by the review process; with two yields the sleeps were
  65 zero / 0 full / 2 parked / 35 partial, and 31 of the 35 partials resumed after exactly 20 writer syscalls. The classifier
  script is not included.
- `--verify` needs an idempotent guest: the workload removes its database files on every exit path, and runs use `--tmp` so
  each run gets a fresh `/tmp`. Pass `--summary-json` a path outside `/tmp`.
- A skid margin of 5000 made runs exceed 900 s; the effective default is 100, and 25 was fastest with no overshoot.

## Reproduce

```sh
./setup.sh
# native, widened window (needs the patched build): see scripts/native3.sh
# Hermit, one run of the assisted case: see scripts/pilot6_worker.sh (KIND=jc100d or jc100)
```
