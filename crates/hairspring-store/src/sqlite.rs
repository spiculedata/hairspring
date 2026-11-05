use crate::checks::{CheckRecord, ConfirmationOutcome, NewConfirmation};
use crate::digest_allowance::DigestUse;
use crate::lineage::*;
use crate::run_mode::mode_of_config;
use crate::sqlite_checks as checks;
use crate::sqlite_explorer as explorer;
use crate::sqlite_lineage as lineage;
use crate::sqlite_read_pool::ReadPool;
use crate::sqlite_run_end as run_end;
use crate::sqlite_schema;
use crate::sqlite_tree_query as query;
use crate::sqlite_trials as trials;
use crate::sqlite_wal_limit::{WalLimit, Writer, JOURNAL_SIZE_LIMIT};
use crate::trials::*;
use crate::types::*;
use crate::{new_id, sha256_hex, FailureFilter, RunFilter, Store};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Backend(e.to_string())
    }
}

/// How long a connection waits for another writer before giving up with `SQLITE_BUSY`.
pub const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Pause between attempts to switch the journal mode while another connection holds the file.
const WAL_RETRY_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);

/// Puts the database in WAL mode. Changing the journal mode takes locks for which SQLite does not always call the busy
/// handler, so concurrent first opens can see `SQLITE_BUSY` even with a busy timeout set; retry until [`BUSY_TIMEOUT`]
/// has passed in total (each attempt waits only for the time that remains). A database already in WAL mode needs no
/// change, which keeps later opens off the lock entirely. A filesystem that cannot do WAL answers with another mode;
/// that is an error rather than a silent downgrade.
fn enable_wal(conn: &Connection) -> Result<()> {
    let deadline = std::time::Instant::now() + BUSY_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        conn.busy_timeout(remaining)?;
        match switch_to_wal(conn) {
            Ok(mode) if is_wal(&mode) => return Ok(()),
            Ok(mode) => {
                return Err(StoreError::Backend(format!(
                    "could not enable WAL journal mode (the database reports `{mode}`); the filesystem may not support it"
                )))
            }
            Err(e) if is_busy(&e) && !remaining.is_zero() => std::thread::sleep(WAL_RETRY_PAUSE),
            Err(e) => return Err(e.into()),
        }
    }
}

/// Returns the journal mode in effect afterwards, changing it only when it is not already WAL.
fn switch_to_wal(conn: &Connection) -> rusqlite::Result<String> {
    let current: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if is_wal(&current) {
        return Ok(current);
    }
    conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
}

fn is_wal(mode: &str) -> bool {
    mode.eq_ignore_ascii_case("wal")
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Read-only connections a store on a database file opens besides its writer, unless told otherwise.
pub const DEFAULT_READ_CONNECTIONS: usize = 4;

/// A store on one SQLite database. Writes go to one connection, behind a mutex, so they run one at a time. A store on a
/// file also holds a few read-only connections ([`DEFAULT_READ_CONNECTIONS`] unless opened with
/// [`SqliteStore::open_with_read_connections`]), so reads run side by side, with each other and with a write, as WAL
/// mode allows. Every read call runs in one read transaction: it sees one committed snapshot, and that snapshot
/// includes every write that had returned before the call began. An in-memory database cannot be shared between
/// connections, so an in-memory store reads and writes through its one connection, as does a store opened with no
/// read connections. A store on a file keeps its WAL file bounded (see `sqlite_wal_limit`).
pub struct SqliteStore {
    // Dropping the store closes the read connections first (see `Drop` below), so the writer is the last connection:
    // it checkpoints the WAL and its close removes the WAL files.
    pub(crate) readers: Option<ReadPool>,
    pub(crate) conn: Mutex<Connection>,
    /// How subtree queries are answered; the store's own rule unless a caller changes it.
    query_rule: Mutex<TreeQueryRule>,
    /// The WAL file of a store on a database file, kept bounded after each use of the writer; `None` in memory.
    pub(crate) wal: Option<WalLimit>,
}

impl Drop for SqliteStore {
    fn drop(&mut self) {
        drop(self.readers.take());
        self.checkpoint_on_close();
    }
}

pub(crate) fn run_from_row(r: &Row<'_>) -> rusqlite::Result<RunRecord> {
    let state: String = r.get("state")?;
    Ok(RunRecord {
        id: r.get("id")?,
        target: r.get("target")?,
        backend: r.get("backend")?,
        state: RunState::parse(&state).unwrap_or(RunState::Error),
        seed: r.get::<_, i64>("seed")? as u64,
        config_json: r.get("config_json")?,
        created_at: r.get("created_at")?,
        started_at: r.get("started_at")?,
        finished_at: r.get("finished_at")?,
        steps: r.get::<_, i64>("steps")? as u64,
        trials: r.get::<_, i64>("trials")? as u64,
        error: r.get("error")?,
        inconclusive: r.get::<_, i64>("inconclusive")? as u64,
        distinct_schedules: r.get::<_, i64>("distinct_schedules")? as u64,
        work: r.get::<_, i64>("work")? as u64,
        mode: r.get("mode")?,
        stopped_at_first_failure: r.get("stopped_at_first_failure")?,
        property_summary_json: r.get("property_summary_json")?,
        catalog_size: r.get::<_, i64>("catalog_size")? as u64,
        ingest_incomplete: r.get("ingest_incomplete")?,
        ingest_attempt: r.get::<_, i64>("ingest_attempt")? as u64,
        build_id: r.get("build_id")?,
    })
}

pub(crate) fn failure_from_row(r: &Row<'_>) -> rusqlite::Result<FailureRecord> {
    Ok(FailureRecord {
        id: r.get("id")?,
        run_id: r.get("run_id")?,
        target: r.get("target")?,
        message: r.get("message")?,
        original_len: r.get::<_, i64>("original_len")? as usize,
        shrunk_len: r.get::<_, i64>("shrunk_len")? as usize,
        bundle_hash: r.get("bundle_hash")?,
        steps_json: r.get("steps_json")?,
        created_at: r.get("created_at")?,
    })
}

pub(crate) fn job_from_row(r: &Row<'_>) -> rusqlite::Result<Job> {
    let status: String = r.get("status")?;
    Ok(Job {
        id: r.get("id")?,
        run_id: r.get("run_id")?,
        kind: r.get("kind")?,
        payload_json: r.get("payload_json")?,
        status: JobStatus::parse(&status).unwrap_or(JobStatus::Failed),
        lease_owner: r.get("lease_owner")?,
        lease_until: r.get("lease_until")?,
        attempts: r.get::<_, i64>("attempts")? as u32,
        last_error: r.get("last_error")?,
        created_at: r.get("created_at")?,
    })
}

impl SqliteStore {
    /// Opens (creating if need be) the database file at `path` with [`DEFAULT_READ_CONNECTIONS`] read connections.
    pub fn open(path: &Path) -> Result<SqliteStore> {
        SqliteStore::open_with_read_connections(path, DEFAULT_READ_CONNECTIONS)
    }

    /// Opens the database file at `path` with `readers` read-only connections besides the writer; with none, reads
    /// share the writer's connection. The writer opens first and runs the open-time checks and the schema migration,
    /// so the read connections only ever see the current schema. A path that names no file (an in-memory or temporary
    /// database) gets no read connections, since another connection would see a different database.
    pub fn open_with_read_connections(path: &Path, readers: usize) -> Result<SqliteStore> {
        let conn = Connection::open(path)?;
        // The timeout must be in place before the first statement that takes a lock: switching to WAL needs one, and
        // without a busy handler it fails at once with "database is locked" when another process is opening the file.
        conn.busy_timeout(BUSY_TIMEOUT)?;
        enable_wal(&conn)?;
        conn.pragma_update(None, "journal_size_limit", JOURNAL_SIZE_LIMIT)?;
        let file = conn
            .path()
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from);
        let mut store = SqliteStore::init(conn)?;
        if let Some(file) = file {
            store.wal = Some(WalLimit::new(&file));
            if readers > 0 {
                store.readers = Some(ReadPool::open(&file, readers)?);
            }
        }
        Ok(store)
    }

    pub fn open_in_memory() -> Result<SqliteStore> {
        SqliteStore::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<SqliteStore> {
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        sqlite_schema::migrate(&conn)?;
        Ok(SqliteStore {
            readers: None,
            conn: Mutex::new(conn),
            query_rule: Mutex::new(TreeQueryRule::StoreDefault),
            wal: None,
        })
    }

    fn rule(&self) -> TreeQueryRule {
        *self.query_rule.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The writer's connection, behind its mutex. Giving it back checks the WAL's size (see `sqlite_wal_limit`).
    pub(crate) fn conn(&self) -> Writer<'_> {
        // A poisoned mutex only means another thread panicked mid-query; SQLite state is still consistent.
        Writer::new(self, self.conn.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Runs `f` on the store's writer connection, so tests can inspect or seed a database with raw SQL. What it commits
    /// is seen by the next read call. Not part of the supported interface.
    #[doc(hidden)]
    pub fn with_connection<R>(&self, f: impl FnOnce(&mut Connection) -> R) -> R {
        f(&mut self.conn())
    }

    /// The plan SQLite chooses for a tree query in one form, one line per step, bound with real values for the given
    /// node. For tests that assert the plan; not part of the supported interface.
    #[doc(hidden)]
    pub fn explain_tree_query(
        &self,
        query: TreeQuery,
        form: TreeQueryForm,
        tree_id: &str,
        node: u64,
    ) -> Result<Vec<String>> {
        self.read(|c| query::explain(c, query, form, tree_id, node))
    }

    /// The form the current rule gives subtree queries on this tree. For tests; not a supported interface.
    #[doc(hidden)]
    pub fn tree_query_form(&self, tree_id: &str) -> Result<TreeQueryForm> {
        let tree = self
            .read(|c| lineage::get_tree(c, tree_id))?
            .ok_or_else(|| StoreError::NotFound(format!("tree {tree_id}")))?;
        Ok(query::form_for(self.rule(), tree.node_count))
    }
}

fn expect_changed(changed: usize, what: &str, id: &str) -> Result<()> {
    if changed == 0 {
        Err(StoreError::NotFound(format!("{what} {id}")))
    } else {
        Ok(())
    }
}

impl SqliteStore {
    /// The steps of the plan SQLite chooses for [`Store::list_failures_page`] with this filter, one per line. For tests
    /// that assert the plan; not part of the supported interface.
    #[doc(hidden)]
    pub fn explain_failures_page(
        &self,
        filter: &FailureFilter,
        offset: u64,
        limit: u32,
    ) -> Result<Vec<String>> {
        self.read(|c| crate::sqlite_listing::explain_failures(c, filter, offset, limit))
    }
}

impl Store for SqliteStore {
    fn create_run(&self, new: &NewRun, now: i64) -> Result<RunRecord> {
        let id = new_id("run");
        let conn = self.conn();
        conn.execute(
            "INSERT INTO runs(id,target,backend,state,seed,config_json,created_at,mode) VALUES (?,?,?,?,?,?,?,?)",
            params![id, new.target, new.backend, RunState::Queued.as_str(), new.seed as i64, new.config_json, now, mode_of_config(&new.config_json)],
        )?;
        Ok(conn.query_row("SELECT * FROM runs WHERE id=?", [&id], run_from_row)?)
    }

    fn get_run(&self, id: &str) -> Result<Option<RunRecord>> {
        self.read(|c| {
            Ok(
                c.query_row("SELECT * FROM runs WHERE id=?", [id], run_from_row)
                    .optional()?,
            )
        })
    }

    fn list_runs(&self, limit: u32) -> Result<Vec<RunRecord>> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare("SELECT * FROM runs ORDER BY created_at DESC, rowid DESC LIMIT ?")?;
            let rows = stmt
                .query_map([limit], run_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    fn list_runs_page(
        &self,
        filter: &RunFilter,
        offset: u64,
        limit: u32,
    ) -> Result<Vec<RunRecord>> {
        self.read(|c| crate::sqlite_listing::list_runs(c, filter, offset, limit))
    }

    fn search(&self, q: &crate::SearchQuery) -> Result<crate::SearchHits> {
        self.read(|c| crate::sqlite_search::run(c, q))
    }

    fn mark_running(&self, id: &str, now: i64) -> Result<()> {
        let n = self.conn().execute("UPDATE runs SET state=?, started_at=COALESCE(started_at,?) WHERE id=? AND state IN ('queued','running')", params![RunState::Running.as_str(), now, id])?;
        expect_changed(n, "run", id)
    }

    fn update_progress(&self, id: &str, steps: u64, trials: u64) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE runs SET steps=?, trials=? WHERE id=?",
            params![steps as i64, trials as i64, id],
        )?;
        expect_changed(n, "run", id)
    }

    fn finish_run(
        &self,
        id: &str,
        state: RunState,
        steps: u64,
        trials: u64,
        error: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE runs SET state=?, steps=?, trials=?, error=?, finished_at=? WHERE id=? AND state IN ('queued','running')",
            params![state.as_str(), steps as i64, trials as i64, error, now, id],
        )?;
        if n == 0 && self.get_run(id)?.is_none() {
            return Err(StoreError::NotFound(format!("run {id}")));
        }
        Ok(n > 0)
    }

    fn cancel_run(&self, id: &str, now: i64) -> Result<bool> {
        run_end::cancel(&mut self.conn(), id, None, now)
    }

    fn cancel_run_with_event(&self, id: &str, event: &NewEvent<'_>, now: i64) -> Result<bool> {
        run_end::cancel(&mut self.conn(), id, Some(event), now)
    }

    fn finish_run_as_error(
        &self,
        id: &str,
        reason: &str,
        event: &NewEvent<'_>,
        now: i64,
    ) -> Result<bool> {
        run_end::finish_as_error(&mut self.conn(), id, reason, event, now)
    }

    fn add_failure(&self, new: &NewFailure, now: i64) -> Result<FailureRecord> {
        let id = new_id("fail");
        let conn = self.conn();
        conn.execute(
            "INSERT INTO failures(id,run_id,target,message,original_len,shrunk_len,bundle_hash,steps_json,created_at) VALUES (?,?,?,?,?,?,?,?,?)",
            params![id, new.run_id, new.target, new.message, new.original_len as i64, new.shrunk_len as i64, new.bundle_hash, new.steps_json, now],
        )?;
        Ok(conn.query_row("SELECT * FROM failures WHERE id=?", [&id], failure_from_row)?)
    }

    fn get_failure(&self, id: &str) -> Result<Option<FailureRecord>> {
        self.read(|c| {
            Ok(
                c.query_row("SELECT * FROM failures WHERE id=?", [id], failure_from_row)
                    .optional()?,
            )
        })
    }

    fn list_failures(&self, run_id: &str) -> Result<Vec<FailureRecord>> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare("SELECT * FROM failures WHERE run_id=? ORDER BY created_at, rowid")?;
            let rows = stmt
                .query_map([run_id], failure_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    fn list_recent_failures(&self, limit: u32) -> Result<Vec<FailureRecord>> {
        self.read(|conn| {
            let mut stmt = conn
                .prepare("SELECT * FROM failures ORDER BY created_at DESC, rowid DESC LIMIT ?")?;
            let rows = stmt
                .query_map([limit], failure_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    fn list_failures_page(
        &self,
        filter: &FailureFilter,
        offset: u64,
        limit: u32,
    ) -> Result<Vec<FailureRecord>> {
        self.read(|c| crate::sqlite_listing::list_failures(c, filter, offset, limit))
    }

    fn retention_scan(&self, cutoff_ms: i64) -> Result<Vec<crate::RetentionCandidate>> {
        self.read(|c| crate::sqlite_retention::scan(c, cutoff_ms))
    }

    fn retention_lock(&self) -> Result<Option<crate::RetentionLock>> {
        let path = self.conn().path().map(str::to_owned);
        crate::sqlite_retention::try_lock(path.as_deref())
    }

    fn retention_delete(
        &self,
        run_ids: &[String],
        cutoff_ms: i64,
        dry_run: bool,
    ) -> Result<crate::RetentionReport> {
        crate::sqlite_retention::delete(&mut self.conn(), run_ids, cutoff_ms, dry_run)
    }

    fn put_blob(&self, data: &[u8]) -> Result<String> {
        let hash = sha256_hex(data);
        self.conn().execute(
            "INSERT OR IGNORE INTO blobs(hash,data) VALUES (?,?)",
            params![hash, data],
        )?;
        Ok(hash)
    }

    fn get_blob(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        self.read(|c| {
            Ok(
                c.query_row("SELECT data FROM blobs WHERE hash=?", [hash], |r| r.get(0))
                    .optional()?,
            )
        })
    }

    fn enqueue_job(&self, run_id: &str, kind: &str, payload_json: &str, now: i64) -> Result<Job> {
        self.jobs_enqueue_job(run_id, kind, payload_json, now)
    }

    fn get_job(&self, id: &str) -> Result<Option<Job>> {
        self.jobs_get_job(id)
    }

    fn lease_job(&self, worker_id: &str, lease_secs: u64, now: i64) -> Result<Option<Job>> {
        self.jobs_lease_job(None, worker_id, lease_secs, now)
    }

    fn lease_run_job(
        &self,
        run_id: &str,
        worker_id: &str,
        lease_secs: u64,
        now: i64,
    ) -> Result<Option<Job>> {
        self.jobs_lease_job(Some(run_id), worker_id, lease_secs, now)
    }

    fn heartbeat_job(&self, id: &str, worker_id: &str, lease_secs: u64, now: i64) -> Result<bool> {
        self.jobs_heartbeat_job(id, worker_id, lease_secs, now)
    }

    fn complete_job(&self, id: &str, worker_id: &str) -> Result<bool> {
        self.jobs_complete_job(id, worker_id)
    }

    fn fail_job(&self, id: &str, worker_id: &str, error: &str, _now: i64) -> Result<bool> {
        self.jobs_fail_job(id, worker_id, error, _now)
    }

    fn fail_expired_jobs(&self, now: i64) -> Result<Vec<ExpiredJob>> {
        self.jobs_fail_expired_jobs(now)
    }

    fn finish_job(
        &self,
        id: &str,
        worker_id: &str,
        result: &JobResult,
        now: i64,
    ) -> Result<FinishOutcome> {
        self.jobs_finish_job(id, worker_id, result, now)
    }

    fn queued_job_count(&self) -> Result<u64> {
        self.jobs_queued_job_count()
    }

    fn leased_jobs(&self, now: i64, limit: u32) -> Result<Vec<LiveLease>> {
        self.jobs_leased_jobs(now, limit)
    }

    fn leased_job_count(&self, now: i64) -> Result<u64> {
        self.jobs_leased_job_count(now)
    }

    fn add_event(&self, run_id: &str, kind: &str, payload_json: &str, now: i64) -> Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO events(run_id,ts,kind,payload_json) VALUES (?,?,?,?)",
            params![run_id, now, kind, payload_json],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn list_events(&self, run_id: &str, after_id: i64, limit: u32) -> Result<Vec<EventRecord>> {
        self.read(|conn| {
            let mut stmt = conn.prepare("SELECT id,run_id,ts,kind,payload_json FROM events WHERE run_id=? AND id>? ORDER BY id LIMIT ?")?;
            let rows = stmt
                .query_map(params![run_id, after_id, limit], |r| {
                    Ok(EventRecord {
                        id: r.get(0)?,
                        run_id: r.get(1)?,
                        ts: r.get(2)?,
                        kind: r.get(3)?,
                        payload_json: r.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    fn ingest_trials(
        &self,
        job_id: &str,
        worker_id: &str,
        worker_trusted: bool,
        batch: &TrialBatch,
        _now: i64,
    ) -> Result<IngestOutcome> {
        trials::ingest(&mut self.conn(), job_id, worker_id, worker_trusted, batch)
    }

    fn count_trials(&self, run_id: &str) -> Result<u64> {
        self.read(|c| trials::count_trials(c, run_id))
    }

    fn get_trial(&self, run_id: &str, seq: u64) -> Result<Option<TrialRecord>> {
        self.read(|c| trials::get_trial(c, run_id, seq))
    }

    fn list_trials(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: u32,
    ) -> Result<Vec<TrialRecord>> {
        self.read(|c| trials::list_trials(c, run_id, after_seq, limit))
    }

    fn list_trial_digests(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: u32,
    ) -> Result<Vec<TrialDigestRow>> {
        self.read(|c| trials::list_trial_digests(c, run_id, after_seq, limit))
    }

    fn list_trial_strip_rows(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: u32,
    ) -> Result<Vec<TrialStripRow>> {
        self.read(|c| trials::list_trial_strip_rows(c, run_id, after_seq, limit))
    }

    fn get_catalog(&self, run_id: &str) -> Result<Vec<CatalogRowRecord>> {
        self.read(|c| trials::get_catalog(c, run_id))
    }

    fn list_run_properties(&self, run_id: &str) -> Result<Vec<RunPropertyRow>> {
        self.read(|c| trials::list_run_properties(c, run_id))
    }

    fn run_contributors(&self, run_id: &str) -> Result<Vec<Contributor>> {
        self.read(|c| trials::run_contributors(c, run_id))
    }

    fn reporters_of(&self, run_id: &str, seeds: &[u64]) -> Result<BTreeMap<u64, String>> {
        self.read(|c| explorer::reporters_of(c, run_id, seeds))
    }

    fn run_digest_use(&self, run_id: &str) -> Result<DigestUse> {
        self.read(|c| explorer::load_digest_use(c, run_id))
    }

    fn run_coverage_json(&self, run_id: &str) -> Result<Option<String>> {
        self.read(|c| explorer::run_coverage_json(c, run_id))
    }

    fn set_worker_trusted(&self, id: &str, trusted: bool) -> Result<()> {
        trials::set_worker_trusted(&self.conn(), id, trusted)
    }

    fn worker_trusted(&self, id: &str) -> Result<Option<bool>> {
        self.read(|c| trials::worker_trusted(c, id))
    }

    fn register_worker_with_secret(
        &self,
        name: &str,
        caps_json: &str,
        secret_hash: &str,
        now: i64,
    ) -> Result<WorkerRecord> {
        let id = new_id("wrk");
        self.conn().execute(
            "INSERT INTO workers(id,name,caps_json,registered_at,last_seen,secret_hash) VALUES (?,?,?,?,?,?)",
            params![id, name, caps_json, now, now, secret_hash],
        )?;
        Ok(WorkerRecord {
            id,
            name: name.to_string(),
            caps_json: caps_json.to_string(),
            registered_at: now,
            last_seen: now,
            trusted: false,
        })
    }

    fn add_confirmation(&self, new: &NewConfirmation, now: i64) -> Result<ConfirmationOutcome> {
        checks::add(&mut self.conn(), new, now)
    }

    fn list_failure_checks(&self, failure_id: &str) -> Result<Vec<CheckRecord>> {
        self.read(|c| checks::list(c, failure_id))
    }

    fn list_checks_of_failures(
        &self,
        failure_ids: &[&str],
    ) -> Result<BTreeMap<String, Vec<CheckRecord>>> {
        self.read(|c| checks::list_many(c, failure_ids))
    }

    fn put_run_identity(&self, identity: &RunIdentity) -> Result<IdentityOutcome> {
        lineage::put_identity(&mut self.conn(), identity)
    }

    fn get_run_identity(&self, run_id: &str) -> Result<Option<RunIdentity>> {
        self.read(|c| lineage::get_identity(c, run_id))
    }

    fn create_tree(&self, new: &NewTree, now: i64) -> Result<TreeOutcome> {
        lineage::create_tree(&mut self.conn(), new, now)
    }

    fn get_tree(&self, id: &str) -> Result<Option<TreeRecord>> {
        self.read(|c| lineage::get_tree(c, id))
    }

    fn tree_of_run(&self, run_id: &str) -> Result<Option<String>> {
        self.read(|c| lineage::tree_of_run(c, run_id))
    }

    fn add_tree_node(&self, tree_id: &str, new: &NewNode, now: i64) -> Result<NodeOutcome> {
        lineage::add_node(&mut self.conn(), tree_id, new, now)
    }

    fn get_tree_node(&self, tree_id: &str, node: u64) -> Result<Option<NodeRecord>> {
        self.read(|c| lineage::get_node(c, tree_id, node))
    }

    fn tree_node_of_trial(&self, run_id: &str, seq: u64) -> Result<Option<NodeRecord>> {
        self.read(|c| lineage::node_of_trial(c, run_id, seq))
    }

    fn set_node_verified(&self, tree_id: &str, node: u64, verified: Verified) -> Result<()> {
        lineage::set_verified(&self.conn(), tree_id, node, verified)
    }

    fn set_tree_query_rule(&self, rule: TreeQueryRule) {
        *self.query_rule.lock().unwrap_or_else(|e| e.into_inner()) = rule;
    }

    fn tree_query_rule(&self) -> TreeQueryRule {
        self.rule()
    }

    fn subtree_descendants(
        &self,
        tree_id: &str,
        node: u64,
        after: Option<u64>,
        limit: u32,
    ) -> Result<NodePage> {
        self.read(|c| query::descendants(c, self.rule(), tree_id, node, after, limit))
    }

    fn subtree_tally(&self, tree_id: &str, node: u64) -> Result<Vec<VerdictCount>> {
        self.read(|c| query::tally(c, self.rule(), tree_id, node))
    }

    fn subtree_first_failing(&self, tree_id: &str, node: u64) -> Result<Option<u64>> {
        self.read(|c| query::first_failing(c, self.rule(), tree_id, node))
    }

    fn tree_children(
        &self,
        tree_id: &str,
        node: u64,
        after: Option<u64>,
        limit: u32,
    ) -> Result<NodePage> {
        self.read(|c| query::children(c, tree_id, node, after, limit))
    }

    fn tree_ancestors(&self, tree_id: &str, node: u64) -> Result<Vec<NodeRecord>> {
        self.read(|c| query::ancestors(c, tree_id, node))
    }

    fn add_tree_branch(&self, new: &NewBranch, now: i64) -> Result<BranchOutcome> {
        lineage::add_branch(&mut self.conn(), new, now)
    }

    fn get_tree_branch(&self, run_id: &str) -> Result<Option<BranchRecord>> {
        self.read(|c| lineage::get_branch(c, run_id))
    }

    fn find_tree_branch(&self, tree_id: &str, command_hash: &str) -> Result<Option<BranchRecord>> {
        self.read(|c| lineage::find_branch(c, tree_id, command_hash))
    }

    fn record_branch_verify(&self, run_id: &str, matched: bool) -> Result<()> {
        lineage::record_verify(&self.conn(), run_id, matched)
    }

    fn set_worker_secret_hash(&self, id: &str, hash: &str) -> Result<()> {
        trials::set_worker_secret_hash(&self.conn(), id, hash)
    }

    fn worker_secret_hash(&self, id: &str) -> Result<Option<String>> {
        self.read(|c| trials::worker_secret_hash(c, id))
    }

    fn register_worker(&self, name: &str, caps_json: &str, now: i64) -> Result<WorkerRecord> {
        let id = new_id("wrk");
        self.conn().execute(
            "INSERT INTO workers(id,name,caps_json,registered_at,last_seen) VALUES (?,?,?,?,?)",
            params![id, name, caps_json, now, now],
        )?;
        Ok(WorkerRecord {
            id,
            name: name.to_string(),
            caps_json: caps_json.to_string(),
            registered_at: now,
            last_seen: now,
            trusted: false,
        })
    }

    fn touch_worker(&self, id: &str, now: i64) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE workers SET last_seen=? WHERE id=?",
            params![now, id],
        )?;
        expect_changed(n, "worker", id)
    }

    fn worker_count(&self) -> Result<u64> {
        self.read(|c| {
            Ok(c.query_row("SELECT COUNT(*) FROM workers", [], |r| r.get::<_, i64>(0))? as u64)
        })
    }

    fn prune_workers(&self, before: i64) -> Result<u64> {
        Ok(self
            .conn()
            .execute("DELETE FROM workers WHERE last_seen < ?", [before])? as u64)
    }

    fn list_workers(&self) -> Result<Vec<WorkerRecord>> {
        self.read(|conn| {
            let mut stmt = conn.prepare("SELECT id,name,caps_json,registered_at,last_seen,trusted FROM workers ORDER BY registered_at, rowid")?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(WorkerRecord {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        caps_json: r.get(2)?,
                        registered_at: r.get(3)?,
                        last_seen: r.get(4)?,
                        trusted: r.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }
}
