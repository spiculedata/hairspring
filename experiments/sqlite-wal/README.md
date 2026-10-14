# SQLite WAL-reset workload

Workload for reproducing the SQLite WAL-reset data race (3.7.0 to 3.51.2, fixed in 3.51.3) under Hairspring's
Hermit backend. See [docs/showcase-sqlite-wal-reset.md](../../docs/showcase-sqlite-wal-reset.md) for the plan and results.

- `walwork.c`: concurrent writers plus a bounded checkpointer on one WAL database. Property: no committed write is
  lost and `PRAGMA integrity_check` is `ok`. Prints `HAIRSPRING_VIOLATION: ...` and exits 1 on failure.
- `setup.sh`: builds against SQLite 3.51.2 and 3.51.3 amalgamations.
- `spike.sh`: runs N Hermit sched-seeds per version (`SEEDS`, `TMO` env vars). Needs the Hermit fork on x86-64 Linux.

The checkpointer is bounded and yields: an unbounded spinning checkpointer starves writers under Hermit's
deterministic scheduler and the run never finishes.
