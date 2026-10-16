//! SQLite schema versions and the migration between them.
//!
//! A database is created at version 1 and then migrated like any older one, so a fresh database and a migrated one
//! cannot differ. Migration runs inside one `BEGIN IMMEDIATE` transaction that re-reads the version, so several
//! processes opening the same old file at once migrate it exactly once and none of them sees a half-migrated file.
use crate::types::{Result, StoreError};
use rusqlite::Connection;

pub(crate) const SCHEMA_VERSION: i64 = 2;

const SCHEMA_V1: &str = "
CREATE TABLE runs (
  id TEXT PRIMARY KEY, target TEXT NOT NULL, backend TEXT NOT NULL, state TEXT NOT NULL,
  seed INTEGER NOT NULL, config_json TEXT NOT NULL, created_at INTEGER NOT NULL,
  started_at INTEGER, finished_at INTEGER, steps INTEGER NOT NULL DEFAULT 0,
  trials INTEGER NOT NULL DEFAULT 0, error TEXT);
CREATE INDEX runs_created ON runs(created_at);
CREATE TABLE failures (
  id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), target TEXT NOT NULL, message TEXT NOT NULL,
  original_len INTEGER NOT NULL, shrunk_len INTEGER NOT NULL, bundle_hash TEXT NOT NULL,
  steps_json TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX failures_run ON failures(run_id);
CREATE TABLE blobs (hash TEXT PRIMARY KEY, data BLOB NOT NULL);
CREATE TABLE jobs (
  id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), kind TEXT NOT NULL, payload_json TEXT NOT NULL,
  status TEXT NOT NULL, lease_owner TEXT, lease_until INTEGER, attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT, created_at INTEGER NOT NULL);
CREATE INDEX jobs_runnable ON jobs(status, created_at);
CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL, ts INTEGER NOT NULL,
  kind TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE INDEX events_run ON events(run_id, id);
CREATE TABLE workers (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, caps_json TEXT NOT NULL,
  registered_at INTEGER NOT NULL, last_seen INTEGER NOT NULL);
";

/// Version 2: run statistics (including the inconclusive count), the run catalog, per-property statistics, one row per
/// judged trial, and an administrator-set trust flag on workers. Every new column has a default, so old rows read
/// back with zero counts.
const MIGRATE_V1_TO_V2: &str = "
ALTER TABLE runs ADD COLUMN inconclusive INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN distinct_schedules INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN work INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN mode TEXT NOT NULL DEFAULT 'search';
ALTER TABLE runs ADD COLUMN stopped_at_first_failure INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN property_summary_json TEXT;
ALTER TABLE runs ADD COLUMN catalog_size INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN ingest_incomplete INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN ingest_attempt INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN kept_bytes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN build_id TEXT;
ALTER TABLE workers ADD COLUMN trusted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE workers ADD COLUMN secret_hash TEXT;
CREATE TABLE run_catalog (
  run_id TEXT NOT NULL REFERENCES runs(id), idx INTEGER NOT NULL, prop_id TEXT NOT NULL, kind TEXT NOT NULL,
  must_hit INTEGER NOT NULL, msg TEXT NOT NULL, loc_file TEXT, loc_line INTEGER, loc_func TEXT,
  decided INTEGER NOT NULL,
  PRIMARY KEY (run_id, idx), UNIQUE (run_id, prop_id));
CREATE TABLE run_properties (
  run_id TEXT NOT NULL REFERENCES runs(id), idx INTEGER NOT NULL, status TEXT NOT NULL,
  trials INTEGER NOT NULL, trials_rated INTEGER NOT NULL, trials_unreported INTEGER NOT NULL,
  trials_degraded INTEGER NOT NULL, trials_perturbing INTEGER NOT NULL, trials_conflicted INTEGER NOT NULL,
  trials_inconclusive_with_failures INTEGER NOT NULL, trials_claimable INTEGER NOT NULL,
  trials_hit INTEGER NOT NULL, trials_true INTEGER NOT NULL, trials_false INTEGER NOT NULL,
  evals INTEGER NOT NULL, evals_true INTEGER NOT NULL, evals_false INTEGER NOT NULL,
  rate_basis TEXT, rate_k INTEGER, rate_n INTEGER, first_failure_seed INTEGER,
  PRIMARY KEY (run_id, idx));
CREATE TABLE trials (
  run_id TEXT NOT NULL REFERENCES runs(id), seq INTEGER NOT NULL, seed INTEGER NOT NULL,
  verdict TEXT NOT NULL, reason TEXT NOT NULL, exit_code INTEGER, signal INTEGER,
  timed_out INTEGER NOT NULL, degraded INTEGER NOT NULL, perturbing INTEGER NOT NULL,
  report_missing INTEGER NOT NULL, rated INTEGER NOT NULL, claimable INTEGER NOT NULL,
  behaviour_hash TEXT, schedule_fp TEXT, syscalls INTEGER NOT NULL, work_ops INTEGER NOT NULL,
  elapsed_ms INTEGER NOT NULL, worker_id TEXT NOT NULL, worker_trusted INTEGER NOT NULL,
  attempt INTEGER NOT NULL, kept TEXT NOT NULL, detail_ref TEXT, output_ref TEXT, digest BLOB NOT NULL,
  PRIMARY KEY (run_id, seq));
CREATE INDEX trials_seed ON trials(run_id, seed);
CREATE INDEX trials_verdict ON trials(run_id, verdict);
CREATE INDEX trials_schedule ON trials(run_id, schedule_fp);
";

fn user_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

fn unsupported(version: i64) -> StoreError {
    StoreError::Backend(format!(
        "database schema version {version} is not supported (expected {SCHEMA_VERSION})"
    ))
}

/// Databases created by an earlier build of version 2 have no `workers.secret_hash`; add it when missing.
fn ensure_secret_column(conn: &Connection) -> Result<()> {
    let present: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('workers') WHERE name='secret_hash'",
        [],
        |r| r.get(0),
    )?;
    if present == 0 {
        conn.execute_batch("ALTER TABLE workers ADD COLUMN secret_hash TEXT")?;
    }
    Ok(())
}

fn upgrade(conn: &Connection, from: i64) -> Result<()> {
    if from == 0 {
        conn.execute_batch(SCHEMA_V1)?;
    }
    if from <= 1 {
        conn.execute_batch(MIGRATE_V1_TO_V2)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

/// Brings the database to [`SCHEMA_VERSION`], creating it if it is empty. A database from a newer release is
/// refused and left untouched.
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let outcome = match user_version(conn) {
        Ok(v) if v == SCHEMA_VERSION => ensure_secret_column(conn),
        Ok(v) if (0..SCHEMA_VERSION).contains(&v) => upgrade(conn, v),
        Ok(v) => Err(unsupported(v)),
        Err(e) => Err(e),
    };
    match outcome {
        Ok(()) => conn.execute_batch("COMMIT").map_err(StoreError::from),
        Err(e) => {
            // The original error is the useful one; a failed rollback leaves nothing worse than the open transaction.
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}
