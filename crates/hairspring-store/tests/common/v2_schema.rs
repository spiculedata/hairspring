//! The version 2 schemas exactly as the previous release left them (version 1 plus its migration), written out here
//! independently of the production migration so tests can build a genuine version 2 database and check that it
//! migrates to version 3 in place.
#![allow(dead_code)]
use super::v1_schema::{POSTGRES_V1, SQLITE_V1};

const SQLITE_V1_TO_V2: &str = "
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

const POSTGRES_V1_TO_V2: &str = "
ALTER TABLE runs ADD COLUMN inconclusive BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN distinct_schedules BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN work BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN mode TEXT NOT NULL DEFAULT 'search';
ALTER TABLE runs ADD COLUMN stopped_at_first_failure BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE runs ADD COLUMN property_summary_json TEXT;
ALTER TABLE runs ADD COLUMN catalog_size BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN ingest_incomplete BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE runs ADD COLUMN ingest_attempt BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN kept_bytes BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN build_id TEXT;
ALTER TABLE workers ADD COLUMN trusted BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE workers ADD COLUMN secret_hash TEXT;
CREATE TABLE run_catalog (
  run_id TEXT NOT NULL REFERENCES runs(id), idx INTEGER NOT NULL, prop_id TEXT NOT NULL, kind TEXT NOT NULL,
  must_hit BOOLEAN NOT NULL, msg TEXT NOT NULL, loc_file TEXT, loc_line BIGINT, loc_func TEXT,
  decided BOOLEAN NOT NULL,
  PRIMARY KEY (run_id, idx), UNIQUE (run_id, prop_id));
CREATE TABLE run_properties (
  run_id TEXT NOT NULL REFERENCES runs(id), idx INTEGER NOT NULL, status TEXT NOT NULL,
  trials BIGINT NOT NULL, trials_rated BIGINT NOT NULL, trials_unreported BIGINT NOT NULL,
  trials_degraded BIGINT NOT NULL, trials_perturbing BIGINT NOT NULL, trials_conflicted BIGINT NOT NULL,
  trials_inconclusive_with_failures BIGINT NOT NULL, trials_claimable BIGINT NOT NULL,
  trials_hit BIGINT NOT NULL, trials_true BIGINT NOT NULL, trials_false BIGINT NOT NULL,
  evals BIGINT NOT NULL, evals_true BIGINT NOT NULL, evals_false BIGINT NOT NULL,
  rate_basis TEXT, rate_k BIGINT, rate_n BIGINT, first_failure_seed BIGINT,
  PRIMARY KEY (run_id, idx));
CREATE TABLE trials (
  run_id TEXT NOT NULL REFERENCES runs(id), seq BIGINT NOT NULL, seed BIGINT NOT NULL,
  verdict TEXT NOT NULL, reason TEXT NOT NULL, exit_code INTEGER, signal INTEGER,
  timed_out BOOLEAN NOT NULL, degraded BOOLEAN NOT NULL, perturbing BOOLEAN NOT NULL,
  report_missing BOOLEAN NOT NULL, rated BOOLEAN NOT NULL, claimable BOOLEAN NOT NULL,
  behaviour_hash TEXT, schedule_fp TEXT, syscalls BIGINT NOT NULL, work_ops BIGINT NOT NULL,
  elapsed_ms BIGINT NOT NULL, worker_id TEXT NOT NULL, worker_trusted BOOLEAN NOT NULL,
  attempt BIGINT NOT NULL, kept TEXT NOT NULL, detail_ref TEXT, output_ref TEXT, digest BYTEA NOT NULL,
  PRIMARY KEY (run_id, seq));
CREATE INDEX trials_seed ON trials(run_id, seed);
CREATE INDEX trials_verdict ON trials(run_id, verdict);
CREATE INDEX trials_schedule ON trials(run_id, schedule_fp);
";

/// The whole SQLite version 2 schema (apply, then set `PRAGMA user_version = 2`).
pub fn sqlite_v2() -> String {
    format!("{SQLITE_V1}{SQLITE_V1_TO_V2}")
}

/// The whole PostgreSQL version 2 schema (apply inside the target schema, then record version 2).
pub fn postgres_v2() -> String {
    format!("{POSTGRES_V1}{POSTGRES_V1_TO_V2}")
}
