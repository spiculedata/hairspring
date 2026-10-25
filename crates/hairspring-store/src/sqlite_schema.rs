//! SQLite schema versions and the migration between them.
//!
//! A database is created at version 1 and then migrated like any older one, so a fresh database and a migrated one
//! cannot differ. Migration runs inside one `BEGIN IMMEDIATE` transaction that re-reads the version, so several
//! processes opening the same old file at once migrate it exactly once and none of them sees a half-migrated file.
use crate::types::{Result, StoreError};
use rusqlite::Connection;

pub(crate) const SCHEMA_VERSION: i64 = 5;

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

/// Version 3: the execution inputs of a run, trial trees (nodes with a materialised path, branch runs). Purely
/// additive: `runs` and `trials` are not touched. `trial_lineage` is a WITHOUT ROWID table so that its primary key is
/// the clustered key and a window of node numbers is one range scan in node order. A path is the LEB128 encoding of
/// the node numbers from the root; SQLite sorts a TEXT value before any BLOB, so the CHECK keeps text out of the column
/// (it would otherwise escape every range).
const MIGRATE_V2_TO_V3: &str = "
CREATE TABLE IF NOT EXISTS run_identity (
  run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, identity_text TEXT NOT NULL,
  identity_hash TEXT NOT NULL, complete INTEGER NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS trees (
  id TEXT PRIMARY KEY REFERENCES runs(id), backend TEXT NOT NULL, target TEXT NOT NULL, identity_hash TEXT,
  node_count INTEGER NOT NULL DEFAULT 0, run_count INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS trial_lineage (
  tree_id TEXT NOT NULL REFERENCES trees(id), node INTEGER NOT NULL, run_id TEXT NOT NULL, seq INTEGER NOT NULL,
  parent_node INTEGER, depth INTEGER NOT NULL, path BLOB NOT NULL, verdict TEXT NOT NULL, operator TEXT NOT NULL,
  cut_kind TEXT NOT NULL, cut_at INTEGER NOT NULL, suffix_seed INTEGER, edge_ref TEXT, origin TEXT NOT NULL,
  reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, verified TEXT NOT NULL DEFAULT 'no',
  created_at INTEGER NOT NULL,
  PRIMARY KEY (tree_id, node), UNIQUE (run_id, seq),
  CHECK (typeof(path) = 'blob'),
  CHECK (parent_node IS NULL OR parent_node < node),
  FOREIGN KEY (run_id, seq) REFERENCES trials(run_id, seq),
  FOREIGN KEY (tree_id, parent_node) REFERENCES trial_lineage(tree_id, node)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS lineage_children ON trial_lineage(tree_id, parent_node, node);
CREATE INDEX IF NOT EXISTS lineage_path ON trial_lineage(tree_id, path);
CREATE INDEX IF NOT EXISTS lineage_verdict ON trial_lineage(tree_id, verdict, depth, node);
CREATE INDEX IF NOT EXISTS lineage_edge ON trial_lineage(edge_ref) WHERE edge_ref IS NOT NULL;
CREATE TABLE IF NOT EXISTS tree_branches (
  run_id TEXT PRIMARY KEY REFERENCES runs(id), tree_id TEXT NOT NULL REFERENCES trees(id),
  source_node INTEGER NOT NULL, cut_kind TEXT NOT NULL, cut_at INTEGER NOT NULL, point_hash TEXT NOT NULL,
  command_json TEXT NOT NULL, command_hash TEXT NOT NULL, seed_source TEXT NOT NULL,
  verify_runs INTEGER NOT NULL DEFAULT 0, verify_matches INTEGER NOT NULL DEFAULT 0,
  created_by TEXT NOT NULL, created_at INTEGER NOT NULL, UNIQUE (tree_id, command_hash));
";

/// Version 4: the digest allowance a run has used and the coverage series stored when its job finished. Purely
/// additive, and separate tables on purpose: `runs` and `trials` are not altered, so migrating a database with many
/// trials rewrites nothing. A run without a row has used nothing and has no stored series.
const MIGRATE_V3_TO_V4: &str = "
CREATE TABLE IF NOT EXISTS run_digest_use (
  run_id TEXT PRIMARY KEY REFERENCES runs(id), bytes INTEGER NOT NULL, capped_trials INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS run_coverage (
  run_id TEXT PRIMARY KEY REFERENCES runs(id), series_json TEXT NOT NULL);
";

/// Version 5: the stored results of confirming a failure. Purely additive and a separate table on purpose: `runs`,
/// `failures` and `trials` are not altered, so no existing row is rewritten. A failure with no row has never been
/// confirmed. Rows are written a confirmation at a time (`confirmation_id`, `ordinal`), which makes a retry idempotent.
const MIGRATE_V4_TO_V5: &str = "
CREATE TABLE IF NOT EXISTS failure_checks (
  id TEXT PRIMARY KEY, failure_id TEXT NOT NULL REFERENCES failures(id), confirmation_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL, passed INTEGER NOT NULL,
  failed INTEGER NOT NULL, inconclusive INTEGER NOT NULL, command TEXT NOT NULL, detail_json TEXT NOT NULL,
  artefacts_json TEXT NOT NULL, environment_json TEXT NOT NULL, started_at INTEGER NOT NULL,
  finished_at INTEGER NOT NULL, runner TEXT NOT NULL, runner_trusted INTEGER NOT NULL, created_at INTEGER NOT NULL,
  UNIQUE (confirmation_id, ordinal));
CREATE INDEX IF NOT EXISTS failure_checks_failure ON failure_checks(failure_id, created_at, confirmation_id, ordinal);
";

/// The tables a version 3 database must have.
const V3_TABLES: [&str; 4] = ["run_identity", "trees", "trial_lineage", "tree_branches"];
/// The tables version 4 added.
const V4_TABLES: [&str; 2] = ["run_digest_use", "run_coverage"];
/// The tables version 5 added.
const V5_TABLES: [&str; 1] = ["failure_checks"];
/// The index version 5 added.
const V5_INDEXES: [&str; 1] = ["failure_checks_failure"];
/// The indexes of `trial_lineage`.
const V3_INDEXES: [&str; 4] = [
    "lineage_children",
    "lineage_path",
    "lineage_verdict",
    "lineage_edge",
];

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

/// A current database is only looked at. Nothing is created or altered, so opening needs no more than the right to
/// write rows; a database that lost a table or an index is reported, not repaired (the children query names its
/// index, so a missing one would otherwise fail later, at the first read).
fn verify_current(conn: &Connection) -> Result<()> {
    let tables: Vec<&str> = V3_TABLES
        .into_iter()
        .chain(V4_TABLES)
        .chain(V5_TABLES)
        .collect();
    let indexes: Vec<&str> = V3_INDEXES.into_iter().chain(V5_INDEXES).collect();
    verify_objects(conn, SCHEMA_VERSION, &tables, &indexes)
}

/// A version 3 database is looked at the same way before version 4 is added: the upgrade does not quietly recreate
/// a version 3 table or index that went missing, on either backend.
fn verify_v3(conn: &Connection) -> Result<()> {
    verify_objects(conn, 3, &V3_TABLES, &V3_INDEXES)
}

/// And a version 4 database before version 5 is added.
fn verify_v4(conn: &Connection) -> Result<()> {
    let tables: Vec<&str> = V3_TABLES.into_iter().chain(V4_TABLES).collect();
    verify_objects(conn, 4, &tables, &V3_INDEXES)
}

fn verify_objects(
    conn: &Connection,
    version: i64,
    tables: &[&str],
    indexes: &[&str],
) -> Result<()> {
    let wanted = tables
        .iter()
        .map(|t| ("table", *t))
        .chain(indexes.iter().map(|i| ("index", *i)));
    for (kind, name) in wanted {
        let present: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type=? AND name=?",
            [kind, name],
            |r| r.get(0),
        )?;
        if present == 0 {
            return Err(StoreError::Backend(format!(
                "database schema version {version} is incomplete: {kind} {name} is missing"
            )));
        }
    }
    Ok(())
}

fn upgrade(conn: &Connection, from: i64) -> Result<()> {
    if from == 0 {
        conn.execute_batch(SCHEMA_V1)?;
    }
    if from <= 1 {
        conn.execute_batch(MIGRATE_V1_TO_V2)?;
    }
    if from == 2 {
        ensure_secret_column(conn)?;
    }
    match from {
        3 => verify_v3(conn)?,
        4 => verify_v4(conn)?,
        _ => conn.execute_batch(MIGRATE_V2_TO_V3)?,
    }
    if from <= 3 {
        conn.execute_batch(MIGRATE_V3_TO_V4)?;
    }
    conn.execute_batch(MIGRATE_V4_TO_V5)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// Brings the database to [`SCHEMA_VERSION`], creating it if it is empty. A database from a newer release is
/// refused and left untouched.
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let outcome = match user_version(conn) {
        Ok(v) if v == SCHEMA_VERSION => verify_current(conn),
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
