//! PostgreSQL schema versions and the migration between them. A schema is created at version 1 and then migrated like
//! any older one, so a fresh schema and a migrated one cannot differ. The caller holds an advisory lock for the whole
//! migration transaction.
use crate::types::{Result, StoreError};
use postgres::Transaction;

pub(crate) const SCHEMA_VERSION: i32 = 4;

/// The tables a version 3 schema must have.
const V3_TABLES: [&str; 4] = ["run_identity", "trees", "trial_lineage", "tree_branches"];
/// The tables version 4 added.
const V4_TABLES: [&str; 2] = ["run_digest_use", "run_coverage"];
/// The indexes of `trial_lineage`.
const V3_INDEXES: [&str; 4] = [
    "lineage_children",
    "lineage_path",
    "lineage_verdict",
    "lineage_edge",
];

const SCHEMA_V1: &str = "
CREATE TABLE runs (
  id TEXT PRIMARY KEY, seq BIGSERIAL, target TEXT NOT NULL, backend TEXT NOT NULL, state TEXT NOT NULL,
  seed BIGINT NOT NULL, config_json TEXT NOT NULL, created_at BIGINT NOT NULL,
  started_at BIGINT, finished_at BIGINT, steps BIGINT NOT NULL DEFAULT 0, trials BIGINT NOT NULL DEFAULT 0, error TEXT);
CREATE INDEX runs_created ON runs(created_at, seq);
CREATE TABLE failures (
  id TEXT PRIMARY KEY, seq BIGSERIAL, run_id TEXT NOT NULL REFERENCES runs(id), target TEXT NOT NULL, message TEXT NOT NULL,
  original_len BIGINT NOT NULL, shrunk_len BIGINT NOT NULL, bundle_hash TEXT NOT NULL, steps_json TEXT NOT NULL, created_at BIGINT NOT NULL);
CREATE INDEX failures_run ON failures(run_id);
CREATE TABLE blobs (hash TEXT PRIMARY KEY, data BYTEA NOT NULL);
CREATE TABLE jobs (
  id TEXT PRIMARY KEY, seq BIGSERIAL, run_id TEXT NOT NULL REFERENCES runs(id), kind TEXT NOT NULL, payload_json TEXT NOT NULL,
  status TEXT NOT NULL, lease_owner TEXT, lease_until BIGINT, attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT, created_at BIGINT NOT NULL);
CREATE INDEX jobs_runnable ON jobs(status, created_at, seq);
CREATE TABLE events (
  id BIGSERIAL PRIMARY KEY, run_id TEXT NOT NULL, ts BIGINT NOT NULL, kind TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE INDEX events_run ON events(run_id, id);
CREATE TABLE workers (
  id TEXT PRIMARY KEY, seq BIGSERIAL, name TEXT NOT NULL, caps_json TEXT NOT NULL, registered_at BIGINT NOT NULL, last_seen BIGINT NOT NULL);
";

/// Version 2: see the SQLite migration of the same name; the two stores keep the same columns.
const MIGRATE_V1_TO_V2: &str = "
ALTER TABLE runs ADD COLUMN IF NOT EXISTS inconclusive BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS distinct_schedules BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS work BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS mode TEXT NOT NULL DEFAULT 'search';
ALTER TABLE runs ADD COLUMN IF NOT EXISTS stopped_at_first_failure BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS property_summary_json TEXT;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS catalog_size BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS ingest_incomplete BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS ingest_attempt BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS kept_bytes BIGINT NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN IF NOT EXISTS build_id TEXT;
ALTER TABLE workers ADD COLUMN IF NOT EXISTS trusted BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE workers ADD COLUMN IF NOT EXISTS secret_hash TEXT;
CREATE TABLE IF NOT EXISTS run_catalog (
  run_id TEXT NOT NULL REFERENCES runs(id), idx INTEGER NOT NULL, prop_id TEXT NOT NULL, kind TEXT NOT NULL,
  must_hit BOOLEAN NOT NULL, msg TEXT NOT NULL, loc_file TEXT, loc_line BIGINT, loc_func TEXT,
  decided BOOLEAN NOT NULL,
  PRIMARY KEY (run_id, idx), UNIQUE (run_id, prop_id));
CREATE TABLE IF NOT EXISTS run_properties (
  run_id TEXT NOT NULL REFERENCES runs(id), idx INTEGER NOT NULL, status TEXT NOT NULL,
  trials BIGINT NOT NULL, trials_rated BIGINT NOT NULL, trials_unreported BIGINT NOT NULL,
  trials_degraded BIGINT NOT NULL, trials_perturbing BIGINT NOT NULL, trials_conflicted BIGINT NOT NULL,
  trials_inconclusive_with_failures BIGINT NOT NULL, trials_claimable BIGINT NOT NULL,
  trials_hit BIGINT NOT NULL, trials_true BIGINT NOT NULL, trials_false BIGINT NOT NULL,
  evals BIGINT NOT NULL, evals_true BIGINT NOT NULL, evals_false BIGINT NOT NULL,
  rate_basis TEXT, rate_k BIGINT, rate_n BIGINT, first_failure_seed BIGINT,
  PRIMARY KEY (run_id, idx));
CREATE TABLE IF NOT EXISTS trials (
  run_id TEXT NOT NULL REFERENCES runs(id), seq BIGINT NOT NULL, seed BIGINT NOT NULL,
  verdict TEXT NOT NULL, reason TEXT NOT NULL, exit_code INTEGER, signal INTEGER,
  timed_out BOOLEAN NOT NULL, degraded BOOLEAN NOT NULL, perturbing BOOLEAN NOT NULL,
  report_missing BOOLEAN NOT NULL, rated BOOLEAN NOT NULL, claimable BOOLEAN NOT NULL,
  behaviour_hash TEXT, schedule_fp TEXT, syscalls BIGINT NOT NULL, work_ops BIGINT NOT NULL,
  elapsed_ms BIGINT NOT NULL, worker_id TEXT NOT NULL, worker_trusted BOOLEAN NOT NULL,
  attempt BIGINT NOT NULL, kept TEXT NOT NULL, detail_ref TEXT, output_ref TEXT, digest BYTEA NOT NULL,
  PRIMARY KEY (run_id, seq));
CREATE INDEX IF NOT EXISTS trials_seed ON trials(run_id, seed);
CREATE INDEX IF NOT EXISTS trials_verdict ON trials(run_id, verdict);
CREATE INDEX IF NOT EXISTS trials_schedule ON trials(run_id, schedule_fp);
";

/// Version 3: see the SQLite migration of the same name; the two stores keep the same columns. `path` is BYTEA, which
/// compares bytewise, so no collation applies to it.
const MIGRATE_V2_TO_V3: &str = "
CREATE TABLE IF NOT EXISTS run_identity (
  run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, identity_text TEXT NOT NULL,
  identity_hash TEXT NOT NULL, complete BOOLEAN NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted BOOLEAN NOT NULL);
CREATE TABLE IF NOT EXISTS trees (
  id TEXT PRIMARY KEY REFERENCES runs(id), backend TEXT NOT NULL, target TEXT NOT NULL, identity_hash TEXT,
  node_count BIGINT NOT NULL DEFAULT 0, run_count BIGINT NOT NULL DEFAULT 1, created_at BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS trial_lineage (
  tree_id TEXT NOT NULL REFERENCES trees(id), node BIGINT NOT NULL, run_id TEXT NOT NULL, seq BIGINT NOT NULL,
  parent_node BIGINT, depth INTEGER NOT NULL, path BYTEA NOT NULL, verdict TEXT NOT NULL, operator TEXT NOT NULL,
  cut_kind TEXT NOT NULL, cut_at BIGINT NOT NULL, suffix_seed BIGINT, edge_ref TEXT, origin TEXT NOT NULL,
  reporter_id TEXT NOT NULL, reporter_trusted BOOLEAN NOT NULL, verified TEXT NOT NULL DEFAULT 'no',
  created_at BIGINT NOT NULL,
  PRIMARY KEY (tree_id, node), UNIQUE (run_id, seq),
  CHECK (parent_node IS NULL OR parent_node < node),
  FOREIGN KEY (run_id, seq) REFERENCES trials(run_id, seq),
  FOREIGN KEY (tree_id, parent_node) REFERENCES trial_lineage(tree_id, node));
CREATE INDEX IF NOT EXISTS lineage_children ON trial_lineage(tree_id, parent_node, node);
CREATE INDEX IF NOT EXISTS lineage_path ON trial_lineage(tree_id, path);
CREATE INDEX IF NOT EXISTS lineage_verdict ON trial_lineage(tree_id, verdict, depth, node);
CREATE INDEX IF NOT EXISTS lineage_edge ON trial_lineage(edge_ref) WHERE edge_ref IS NOT NULL;
CREATE TABLE IF NOT EXISTS tree_branches (
  run_id TEXT PRIMARY KEY REFERENCES runs(id), tree_id TEXT NOT NULL REFERENCES trees(id),
  source_node BIGINT NOT NULL, cut_kind TEXT NOT NULL, cut_at BIGINT NOT NULL, point_hash TEXT NOT NULL,
  command_json TEXT NOT NULL, command_hash TEXT NOT NULL, seed_source TEXT NOT NULL,
  verify_runs BIGINT NOT NULL DEFAULT 0, verify_matches BIGINT NOT NULL DEFAULT 0,
  created_by TEXT NOT NULL, created_at BIGINT NOT NULL, UNIQUE (tree_id, command_hash));
";

/// Version 4: see the SQLite migration of the same name; the two stores keep the same columns. Separate tables on
/// purpose: `runs` and `trials` are not altered, so no existing row is rewritten.
const MIGRATE_V3_TO_V4: &str = "
CREATE TABLE IF NOT EXISTS run_digest_use (
  run_id TEXT PRIMARY KEY REFERENCES runs(id), bytes BIGINT NOT NULL, capped_trials BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS run_coverage (
  run_id TEXT PRIMARY KEY REFERENCES runs(id), series_json TEXT NOT NULL);
";

/// Brings the schema to [`SCHEMA_VERSION`] inside the caller's transaction. A schema from a newer release is refused.
/// A current schema is only looked at: no DDL is issued, so opening it needs no rights beyond reading and writing rows.
pub(crate) fn migrate(tx: &mut Transaction<'_>) -> Result<()> {
    // The version table is created only when it is missing: creating a table that exists is still DDL, and a role that
    // only uses a current schema may not do it.
    let has_version_table = tx
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'schema_version')",
            &[],
        )?
        .get::<_, bool>(0);
    if !has_version_table {
        tx.batch_execute("CREATE TABLE schema_version (version INTEGER NOT NULL)")?;
    }
    let current: Option<i32> = tx
        .query_opt("SELECT version FROM schema_version LIMIT 1", &[])?
        .map(|r| r.get(0));
    match current {
        None => {
            tx.batch_execute(SCHEMA_V1)?;
            tx.batch_execute(MIGRATE_V1_TO_V2)?;
            tx.batch_execute(MIGRATE_V2_TO_V3)?;
            tx.batch_execute(MIGRATE_V3_TO_V4)?;
            tx.execute(
                "INSERT INTO schema_version(version) VALUES ($1)",
                &[&SCHEMA_VERSION],
            )?;
        }
        Some(1) => {
            tx.batch_execute(MIGRATE_V1_TO_V2)?;
            tx.batch_execute(MIGRATE_V2_TO_V3)?;
            tx.batch_execute(MIGRATE_V3_TO_V4)?;
            tx.execute("UPDATE schema_version SET version=$1", &[&SCHEMA_VERSION])?;
        }
        Some(2) => {
            // Schemas created by an earlier build of version 2 have no `workers.secret_hash`. The caller's advisory
            // lock serialises this check with the `ALTER`, which stays idempotent.
            if !has_worker_secret_column(tx)? {
                tx.batch_execute("ALTER TABLE workers ADD COLUMN IF NOT EXISTS secret_hash TEXT")?;
            }
            tx.batch_execute(MIGRATE_V2_TO_V3)?;
            tx.batch_execute(MIGRATE_V3_TO_V4)?;
            tx.execute("UPDATE schema_version SET version=$1", &[&SCHEMA_VERSION])?;
        }
        Some(3) => {
            tx.batch_execute(MIGRATE_V3_TO_V4)?;
            tx.execute("UPDATE schema_version SET version=$1", &[&SCHEMA_VERSION])?;
        }
        Some(v) if v == SCHEMA_VERSION => verify_current(tx)?,
        Some(v) => {
            return Err(StoreError::Backend(format!(
                "database schema version {v} is not supported (expected {SCHEMA_VERSION})"
            )))
        }
    }
    Ok(())
}

/// Reports a current schema that lost one of its tables or a version 3 index instead of repairing it.
fn verify_current(tx: &mut Transaction<'_>) -> Result<()> {
    for table in V3_TABLES.into_iter().chain(V4_TABLES) {
        let row = tx.query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = $1)",
            &[&table],
        )?;
        if !row.get::<_, bool>(0) {
            return Err(StoreError::Backend(format!(
                "database schema version {SCHEMA_VERSION} is incomplete: table {table} is missing"
            )));
        }
    }
    for index in V3_INDEXES {
        let row = tx.query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = current_schema() AND indexname = $1)",
            &[&index],
        )?;
        if !row.get::<_, bool>(0) {
            return Err(StoreError::Backend(format!(
                "database schema version {SCHEMA_VERSION} is incomplete: index {index} is missing"
            )));
        }
    }
    Ok(())
}

/// True when `workers.secret_hash` exists in the schema this connection's `search_path` selects.
fn has_worker_secret_column(tx: &mut Transaction<'_>) -> Result<bool> {
    let row = tx.query_one(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'workers' AND column_name = 'secret_hash')",
        &[],
    )?;
    Ok(row.get(0))
}
