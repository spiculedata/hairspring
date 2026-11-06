//! PostgreSQL schema versions and the migration between them. A schema is created at version 1 and then migrated like
//! any older one, so a fresh schema and a migrated one cannot differ. The caller holds an advisory lock for the whole
//! migration transaction.
use crate::downgrade::{V7_INDEXES, V8_INDEXES, V8_TABLES_DROP_ORDER};
use crate::run_mode::{mode_of_config, SEARCH_MODE, SWEEP_MODE};
use crate::types::{Result, StoreError};
use postgres::Transaction;

pub(crate) const SCHEMA_VERSION: i32 = 8;

/// The tables a version 3 schema must have.
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

/// Version 5: see the SQLite migration of the same name; the two stores keep the same columns. A separate table on
/// purpose: `runs`, `failures` and `trials` are not altered, so no existing row is rewritten.
const MIGRATE_V4_TO_V5: &str = "
CREATE TABLE IF NOT EXISTS failure_checks (
  id TEXT PRIMARY KEY, failure_id TEXT NOT NULL REFERENCES failures(id), confirmation_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL, passed BIGINT NOT NULL,
  failed BIGINT NOT NULL, inconclusive BIGINT NOT NULL, command TEXT NOT NULL, detail_json TEXT NOT NULL,
  artefacts_json TEXT NOT NULL, environment_json TEXT NOT NULL, started_at BIGINT NOT NULL,
  finished_at BIGINT NOT NULL, runner TEXT NOT NULL, runner_trusted BOOLEAN NOT NULL, created_at BIGINT NOT NULL,
  seq BIGSERIAL NOT NULL,
  UNIQUE (confirmation_id, ordinal));
CREATE INDEX IF NOT EXISTS failure_checks_failure ON failure_checks(failure_id, created_at, confirmation_id, ordinal);
";

/// Version 7: see the SQLite migration of the same name; the two stores keep the same indexes. `failures_created` ends
/// in `seq`, the column that orders failures stored in the same millisecond here. `CREATE INDEX` (not `CONCURRENTLY`,
/// which cannot run in a transaction) blocks writes to the table it indexes while it builds; reads carry on. Creating
/// an index needs ownership of the table, so this step must be run by the role that owns the schema's tables.
const MIGRATE_V6_TO_V7: &str = "
CREATE INDEX IF NOT EXISTS failures_created ON failures(created_at, seq);
CREATE INDEX IF NOT EXISTS run_properties_violation ON run_properties(run_id, idx) WHERE status = 'violation';
";

/// Version 8: see the SQLite migration of the same name; the two stores keep the same columns and index. The index ends
/// in `seq`, the column that orders runs created in the same millisecond here (as `runs_created` does). `CREATE INDEX`
/// (not `CONCURRENTLY`, which cannot run in a transaction) blocks writes to `runs` while it builds; reads carry on.
/// Creating the index needs ownership of `runs`, so this step must be run by the role that owns the schema's tables
/// (that role owns the new table too: a role that only reads and writes rows needs `SELECT, INSERT, DELETE` on `run_inputs`
/// granted, as on the other tables, before it can record inputs).
const MIGRATE_V7_TO_V8: &str = "
CREATE INDEX IF NOT EXISTS runs_target_created ON runs(target, created_at, seq);
CREATE TABLE IF NOT EXISTS run_inputs (
  run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, fields_json TEXT NOT NULL,
  missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted BOOLEAN NOT NULL, attempt INTEGER NOT NULL,
  created_at BIGINT NOT NULL);
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
            tx.batch_execute(MIGRATE_V4_TO_V5)?;
            backfill_run_modes(tx)?;
            migrate_v6_to_v7(tx)?;
            migrate_v7_to_v8(tx)?;
            tx.execute(
                "INSERT INTO schema_version(version) VALUES ($1)",
                &[&SCHEMA_VERSION],
            )?;
        }
        Some(1) => {
            tx.batch_execute(MIGRATE_V1_TO_V2)?;
            tx.batch_execute(MIGRATE_V2_TO_V3)?;
            tx.batch_execute(MIGRATE_V3_TO_V4)?;
            tx.batch_execute(MIGRATE_V4_TO_V5)?;
            backfill_run_modes(tx)?;
            migrate_v6_to_v7(tx)?;
            migrate_v7_to_v8(tx)?;
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
            tx.batch_execute(MIGRATE_V4_TO_V5)?;
            backfill_run_modes(tx)?;
            migrate_v6_to_v7(tx)?;
            migrate_v7_to_v8(tx)?;
            tx.execute("UPDATE schema_version SET version=$1", &[&SCHEMA_VERSION])?;
        }
        Some(3) => {
            // A version 3 schema is looked at, as on SQLite, and not repaired by the upgrade.
            verify_objects(tx, 3, &V3_TABLES, &V3_INDEXES)?;
            tx.batch_execute(MIGRATE_V3_TO_V4)?;
            tx.batch_execute(MIGRATE_V4_TO_V5)?;
            backfill_run_modes(tx)?;
            migrate_v6_to_v7(tx)?;
            migrate_v7_to_v8(tx)?;
            tx.execute("UPDATE schema_version SET version=$1", &[&SCHEMA_VERSION])?;
        }
        Some(4) => {
            let tables: Vec<&str> = V3_TABLES.into_iter().chain(V4_TABLES).collect();
            verify_objects(tx, 4, &tables, &V3_INDEXES)?;
            tx.batch_execute(MIGRATE_V4_TO_V5)?;
            backfill_run_modes(tx)?;
            migrate_v6_to_v7(tx)?;
            migrate_v7_to_v8(tx)?;
            tx.execute("UPDATE schema_version SET version=$1", &[&SCHEMA_VERSION])?;
        }
        Some(5) => {
            verify_v5_schema(tx, 5)?;
            backfill_run_modes(tx)?;
            migrate_v6_to_v7(tx)?;
            migrate_v7_to_v8(tx)?;
            tx.execute("UPDATE schema_version SET version=$1", &[&SCHEMA_VERSION])?;
        }
        Some(6) => {
            verify_v5_schema(tx, 6)?;
            migrate_v6_to_v7(tx)?;
            migrate_v7_to_v8(tx)?;
            tx.execute("UPDATE schema_version SET version=$1", &[&SCHEMA_VERSION])?;
        }
        Some(7) => {
            verify_v7(tx, 7)?;
            migrate_v7_to_v8(tx)?;
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

/// Version 6 is data only, as on SQLite: `runs.mode` was never written by an insert, so every run carries the column
/// default. A run whose stored `config_json` names the sweep mode gets it back. The text is read the way a new run's
/// is, in Rust (`config_json` is TEXT, and a cast to JSON would fail the whole migration on one malformed row), so a
/// `config_json` that is not JSON stays a search; a run holding any other mode than the default is left alone.
fn backfill_run_modes(tx: &mut Transaction<'_>) -> Result<()> {
    let candidates = tx.query(
        "SELECT id, config_json FROM runs WHERE mode=$1 AND config_json LIKE $2",
        &[&SEARCH_MODE, &format!("%{SWEEP_MODE}%")],
    )?;
    for row in candidates {
        let (id, config): (String, String) = (row.get(0), row.get(1));
        if mode_of_config(&config) == SWEEP_MODE {
            tx.execute(
                "UPDATE runs SET mode=$1 WHERE id=$2 AND mode=$3",
                &[&SWEEP_MODE, &id, &SEARCH_MODE],
            )?;
        }
    }
    Ok(())
}

/// Reports a current schema that lost one of its tables or a version 3 index instead of repairing it.
fn verify_current(tx: &mut Transaction<'_>) -> Result<()> {
    verify_v7(tx, SCHEMA_VERSION)?;
    verify_objects(tx, SCHEMA_VERSION, &V8_TABLES_DROP_ORDER, &V8_INDEXES)?;
    verify_v8_definitions(tx)
}

/// A version 7 schema (and the version 7 part of a current one): the tables, and the version 7 indexes as the
/// migration created them.
fn verify_v7(tx: &mut Transaction<'_>, version: i32) -> Result<()> {
    verify_v5_schema(tx, version)?;
    verify_objects(tx, version, &[], &V7_INDEXES)?;
    verify_v7_definitions(tx)
}

/// Creates the version 8 table and index and checks what is there afterwards. `IF NOT EXISTS` goes by name, so a table
/// or index of that name made beforehand would be kept: the check refuses one that is not what the migration creates.
fn migrate_v7_to_v8(tx: &mut Transaction<'_>) -> Result<()> {
    tx.batch_execute(MIGRATE_V7_TO_V8)?;
    verify_v8_definitions(tx)
}

/// The columns of `run_inputs`: name, data type as `information_schema` names it, and whether it may be NULL.
const RUN_INPUTS_COLUMNS: [(&str, &str, &str); 8] = [
    ("run_id", "text", "NO"),
    ("scheme", "text", "NO"),
    ("fields_json", "text", "NO"),
    ("missing_json", "text", "NO"),
    ("reporter_id", "text", "NO"),
    ("reporter_trusted", "boolean", "NO"),
    ("attempt", "integer", "NO"),
    ("created_at", "bigint", "NO"),
];

/// The table has these columns, `run_id` alone is its primary key, and `run_id` references `runs`; the index is valid
/// and ready, on `runs`, not unique, on `(target, created_at, seq)`.
fn verify_v8_definitions(tx: &mut Transaction<'_>) -> Result<()> {
    // Everything here is read from the catalogs (`pg_class`, `pg_attribute`, `pg_constraint`, `pg_index`), which show every
    // table whatever the role may do with it. `information_schema` shows only tables the role has a privilege on, and a
    // role without a grant on `run_inputs` would be told the table is not there.
    let columns: Vec<(String, String, String)> = tx
        .query(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), CASE WHEN a.attnotnull THEN 'NO' ELSE 'YES' END \
             FROM pg_attribute a JOIN pg_class t ON t.oid = a.attrelid JOIN pg_namespace n ON n.oid = t.relnamespace \
             WHERE n.nspname = current_schema() AND t.relname = 'run_inputs' AND t.relkind = 'r' \
               AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
            &[],
        )?
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect();
    let wanted: Vec<(String, String, String)> = RUN_INPUTS_COLUMNS
        .iter()
        .map(|(n, t, nullable)| (n.to_string(), t.to_string(), nullable.to_string()))
        .collect();
    // Exactly two constraints (NOT NULL ones, which version 18 lists too, are left out): the primary key on `run_id`,
    // and the reference from `run_id` to `id` of `runs` in this same schema with the default actions. No CHECK, no
    // UNIQUE, no ON DELETE CASCADE. And no index but the primary key's.
    let (constraints, primary, references_runs, indexes): (i64, i64, i64, i64) = {
        let row = tx.query_one(
            "WITH t AS (SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = current_schema() AND c.relname = 'run_inputs' AND c.relkind = 'r') \
             SELECT \
               (SELECT count(*) FROM pg_constraint k, t WHERE k.conrelid = t.oid AND k.contype <> 'n'), \
               (SELECT count(*) FROM pg_constraint k, t WHERE k.conrelid = t.oid AND k.contype = 'p' \
                  AND k.conkey = ARRAY[1]::smallint[]), \
               (SELECT count(*) FROM pg_constraint k, t, pg_class r, pg_namespace rn \
                 WHERE k.conrelid = t.oid AND k.contype = 'f' AND r.oid = k.confrelid AND rn.oid = r.relnamespace \
                   AND rn.nspname = current_schema() AND r.relname = 'runs' AND k.conkey = ARRAY[1]::smallint[] \
                   AND array_length(k.confkey, 1) = 1 \
                   AND (SELECT a.attname FROM pg_attribute a WHERE a.attrelid = k.confrelid AND a.attnum = k.confkey[1]) = 'id' \
                   AND k.confdeltype = 'a' AND k.confupdtype = 'a' AND k.confmatchtype = 's'), \
               (SELECT count(*) FROM pg_index i, t WHERE i.indrelid = t.oid)",
            &[],
        )?;
        (row.get(0), row.get(1), row.get(2), row.get(3))
    };
    if columns != wanted || constraints != 2 || primary != 1 || references_runs != 1 || indexes != 1
    {
        return Err(StoreError::Backend(
            "table run_inputs exists but has a different definition: rename or drop it and open the database again"
                .to_string(),
        ));
    }
    verify_run_inputs_rights(tx)?;
    for (index, table, ending) in V8_DEFINITIONS {
        let row = tx.query_opt(
            "SELECT i.indisvalid AND i.indisready, i.indisunique, t.relname::text, pg_get_indexdef(i.indexrelid) \
             FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid JOIN pg_class t ON t.oid = i.indrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = current_schema() AND c.relname = $1",
            &[&index],
        )?;
        let fits = row.is_some_and(|r| {
            r.get::<_, bool>(0)
                && !r.get::<_, bool>(1)
                && r.get::<_, String>(2) == table
                && r.get::<_, String>(3).ends_with(ending)
        });
        if !fits {
            return Err(StoreError::Backend(format!(
                "index {index} exists but is invalid or has a different definition: drop it and open the database again"
            )));
        }
    }
    Ok(())
}

/// The role must be able to read, write and delete `run_inputs`. Every first batch of trials of an attempt deletes the
/// inputs an earlier attempt left, so a role without the rights could open the database and then fail to ingest trials
/// for any run: it is refused here instead, before anything is written, with the statement that fixes it. The owner and
/// a superuser always pass. (`has_table_privilege` reads the catalogs, so this does not depend on seeing the table.)
fn verify_run_inputs_rights(tx: &mut Transaction<'_>) -> Result<()> {
    let row = tx.query_one(
        "SELECT has_table_privilege(current_user, c.oid, 'SELECT') \
            AND has_table_privilege(current_user, c.oid, 'INSERT') \
            AND has_table_privilege(current_user, c.oid, 'DELETE'), \
                quote_ident(current_user), quote_ident(current_schema()) \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = current_schema() AND c.relname = 'run_inputs' AND c.relkind = 'r'",
        &[],
    )?;
    if row.get::<_, bool>(0) {
        return Ok(());
    }
    let (role, schema): (String, String) = (row.get(1), row.get(2));
    Err(StoreError::Backend(format!(
        "role {role} cannot use table run_inputs: ask the role that owns the schema's tables to run \
         GRANT SELECT, INSERT, DELETE ON {schema}.run_inputs TO {role} \
         and open the database again (nothing was changed)"
    )))
}

/// What each version 8 index must be: its table, and how `pg_get_indexdef` ends for it.
const V8_DEFINITIONS: [(&str, &str, &str); 1] = [(
    "runs_target_created",
    "runs",
    " USING btree (target, created_at, seq)",
)];

/// Creates the version 7 indexes and checks what is there afterwards. `IF NOT EXISTS` goes by name, so an index an
/// operator built beforehand (with `CREATE INDEX CONCURRENTLY`, say) is kept, and one with the same name but another
/// definition, or one left invalid by a failed concurrent build, would be kept too: the check refuses both.
fn migrate_v6_to_v7(tx: &mut Transaction<'_>) -> Result<()> {
    tx.batch_execute(MIGRATE_V6_TO_V7)?;
    verify_v7_definitions(tx)
}

/// What each version 7 index must be: its table, and how `pg_get_indexdef` ends for it.
const V7_DEFINITIONS: [(&str, &str, &str); 2] = [
    (
        "failures_created",
        "failures",
        " USING btree (created_at, seq)",
    ),
    (
        "run_properties_violation",
        "run_properties",
        " USING btree (run_id, idx) WHERE (status = 'violation'::text)",
    ),
];

/// Each version 7 index must be valid and ready (a failed `CREATE INDEX CONCURRENTLY` leaves one that is neither), on
/// its table, not unique, and with the columns and predicate the migration creates.
fn verify_v7_definitions(tx: &mut Transaction<'_>) -> Result<()> {
    for (index, table, ending) in V7_DEFINITIONS {
        let row = tx.query_opt(
            "SELECT i.indisvalid AND i.indisready, i.indisunique, t.relname::text, pg_get_indexdef(i.indexrelid) \
             FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid JOIN pg_class t ON t.oid = i.indrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = current_schema() AND c.relname = $1",
            &[&index],
        )?;
        let fits = row.is_some_and(|r| {
            r.get::<_, bool>(0)
                && !r.get::<_, bool>(1)
                && r.get::<_, String>(2) == table
                && r.get::<_, String>(3).ends_with(ending)
        });
        if !fits {
            return Err(StoreError::Backend(format!(
                "index {index} exists but is invalid or has a different definition: drop it and open the database again"
            )));
        }
    }
    Ok(())
}

/// Versions 5 and 6 have the same tables and indexes (version 6 only rewrites data); version 7 adds indexes to them.
fn verify_v5_schema(tx: &mut Transaction<'_>, version: i32) -> Result<()> {
    let tables: Vec<&str> = V3_TABLES
        .into_iter()
        .chain(V4_TABLES)
        .chain(V5_TABLES)
        .collect();
    let indexes: Vec<&str> = V3_INDEXES.into_iter().chain(V5_INDEXES).collect();
    verify_objects(tx, version, &tables, &indexes)?;
    verify_checks_seq(tx)
}

/// A development database created at an earlier draft of version 5 has `failure_checks` without `seq`. It opens, but
/// every read of the checks would fail with an opaque database error, so it is refused here with the way out.
fn verify_checks_seq(tx: &mut Transaction<'_>) -> Result<()> {
    let row = tx.query_one(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'failure_checks' AND column_name = 'seq')",
        &[],
    )?;
    if row.get::<_, bool>(0) {
        return Ok(());
    }
    Err(StoreError::Backend(
        "database schema version 5 is incomplete: failure_checks has no `seq` column (it was created by an earlier draft of version 5). \
         It holds only stored confirmation checks. In this schema run \
         `DROP TABLE failure_checks; UPDATE schema_version SET version = 4;` and open the database again: it migrates back to version 5. \
         Or reset the development database"
            .to_string(),
    ))
}

fn verify_objects(
    tx: &mut Transaction<'_>,
    version: i32,
    tables: &[&str],
    indexes: &[&str],
) -> Result<()> {
    for table in tables {
        // From `pg_class`, which lists a table whatever the role may do with it (`information_schema` hides a table the
        // role has no privilege on, and the open would be refused for a table that is there).
        let row = tx.query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = current_schema() AND c.relname = $1 AND c.relkind IN ('r', 'p'))",
            &[&table],
        )?;
        if !row.get::<_, bool>(0) {
            return Err(StoreError::Backend(format!(
                "database schema version {version} is incomplete: table {table} is missing"
            )));
        }
    }
    for index in indexes {
        let row = tx.query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = current_schema() AND indexname = $1)",
            &[&index],
        )?;
        if !row.get::<_, bool>(0) {
            return Err(StoreError::Backend(format!(
                "database schema version {version} is incomplete: index {index} is missing"
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
