use crate::lineage::*;
use crate::sqlite_lineage as lineage;
use crate::sqlite_schema;
use crate::sqlite_tree_query as query;
use crate::sqlite_trials as trials;
use crate::trials::*;
use crate::types::*;
use crate::{new_id, sha256_hex, Store, MAX_JOB_ATTEMPTS};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Backend(e.to_string())
    }
}

/// How long a connection waits for another writer before giving up with `SQLITE_BUSY`.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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

pub struct SqliteStore {
    conn: Mutex<Connection>,
    /// How subtree queries are answered; the store's own rule unless a caller changes it.
    query_rule: Mutex<TreeQueryRule>,
}

fn run_from_row(r: &Row<'_>) -> rusqlite::Result<RunRecord> {
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

fn failure_from_row(r: &Row<'_>) -> rusqlite::Result<FailureRecord> {
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

fn job_from_row(r: &Row<'_>) -> rusqlite::Result<Job> {
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
    pub fn open(path: &Path) -> Result<SqliteStore> {
        let conn = Connection::open(path)?;
        // The timeout must be in place before the first statement that takes a lock: switching to WAL needs one, and
        // without a busy handler it fails at once with "database is locked" when another process is opening the file.
        conn.busy_timeout(BUSY_TIMEOUT)?;
        enable_wal(&conn)?;
        SqliteStore::init(conn)
    }

    pub fn open_in_memory() -> Result<SqliteStore> {
        SqliteStore::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<SqliteStore> {
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        sqlite_schema::migrate(&conn)?;
        Ok(SqliteStore {
            conn: Mutex::new(conn),
            query_rule: Mutex::new(TreeQueryRule::StoreDefault),
        })
    }

    fn rule(&self) -> TreeQueryRule {
        *self.query_rule.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A poisoned mutex only means another thread panicked mid-query; SQLite state is still consistent.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Runs `f` on the store's own connection, so tests can inspect or seed an in-memory database with raw SQL. Not
    /// part of the supported interface.
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
        query::explain(&self.conn(), query, form, tree_id, node)
    }

    /// The form the current rule gives subtree queries on this tree. For tests; not a supported interface.
    #[doc(hidden)]
    pub fn tree_query_form(&self, tree_id: &str) -> Result<TreeQueryForm> {
        let tree = lineage::get_tree(&self.conn(), tree_id)?
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

impl Store for SqliteStore {
    fn create_run(&self, new: &NewRun, now: i64) -> Result<RunRecord> {
        let id = new_id("run");
        let conn = self.conn();
        conn.execute(
            "INSERT INTO runs(id,target,backend,state,seed,config_json,created_at) VALUES (?,?,?,?,?,?,?)",
            params![id, new.target, new.backend, RunState::Queued.as_str(), new.seed as i64, new.config_json, now],
        )?;
        Ok(conn.query_row("SELECT * FROM runs WHERE id=?", [&id], run_from_row)?)
    }

    fn get_run(&self, id: &str) -> Result<Option<RunRecord>> {
        Ok(self
            .conn()
            .query_row("SELECT * FROM runs WHERE id=?", [id], run_from_row)
            .optional()?)
    }

    fn list_runs(&self, limit: u32) -> Result<Vec<RunRecord>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT * FROM runs ORDER BY created_at DESC, rowid DESC LIMIT ?")?;
        let rows = stmt
            .query_map([limit], run_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
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
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let n = tx.execute("UPDATE runs SET state='cancelled', finished_at=? WHERE id=? AND state IN ('queued','running')", params![now, id])?;
        if n > 0 {
            tx.execute("UPDATE jobs SET status='failed', last_error='cancelled' WHERE run_id=? AND status='queued'", [id])?;
        }
        tx.commit()?;
        Ok(n > 0)
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
        Ok(self
            .conn()
            .query_row("SELECT * FROM failures WHERE id=?", [id], failure_from_row)
            .optional()?)
    }

    fn list_failures(&self, run_id: &str) -> Result<Vec<FailureRecord>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT * FROM failures WHERE run_id=? ORDER BY created_at, rowid")?;
        let rows = stmt
            .query_map([run_id], failure_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn list_recent_failures(&self, limit: u32) -> Result<Vec<FailureRecord>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT * FROM failures ORDER BY created_at DESC, rowid DESC LIMIT ?")?;
        let rows = stmt
            .query_map([limit], failure_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
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
        Ok(self
            .conn()
            .query_row("SELECT data FROM blobs WHERE hash=?", [hash], |r| r.get(0))
            .optional()?)
    }

    fn enqueue_job(&self, run_id: &str, kind: &str, payload_json: &str, now: i64) -> Result<Job> {
        let id = new_id("job");
        let conn = self.conn();
        conn.execute(
            "INSERT INTO jobs(id,run_id,kind,payload_json,status,created_at) VALUES (?,?,?,?,?,?)",
            params![
                id,
                run_id,
                kind,
                payload_json,
                JobStatus::Queued.as_str(),
                now
            ],
        )?;
        Ok(conn.query_row("SELECT * FROM jobs WHERE id=?", [&id], job_from_row)?)
    }

    fn get_job(&self, id: &str) -> Result<Option<Job>> {
        Ok(self
            .conn()
            .query_row("SELECT * FROM jobs WHERE id=?", [id], job_from_row)
            .optional()?)
    }

    fn lease_job(&self, worker_id: &str, lease_secs: u64, now: i64) -> Result<Option<Job>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let id: Option<String> = tx
            .query_row(
                "SELECT id FROM jobs WHERE (status='queued' OR (status='running' AND lease_until < ?)) AND attempts < ? ORDER BY created_at, rowid LIMIT 1",
                params![now, MAX_JOB_ATTEMPTS],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            tx.commit()?;
            return Ok(None);
        };
        let until = now.saturating_add((lease_secs as i64).saturating_mul(1000));
        tx.execute(
            "UPDATE jobs SET status='running', lease_owner=?, lease_until=?, attempts=attempts+1 WHERE id=?",
            params![worker_id, until, id],
        )?;
        let job = tx.query_row("SELECT * FROM jobs WHERE id=?", [&id], job_from_row)?;
        tx.commit()?;
        Ok(Some(job))
    }

    fn heartbeat_job(&self, id: &str, worker_id: &str, lease_secs: u64, now: i64) -> Result<bool> {
        let until = now.saturating_add((lease_secs as i64).saturating_mul(1000));
        let n = self.conn().execute(
            "UPDATE jobs SET lease_until=? WHERE id=? AND status='running' AND lease_owner=?",
            params![until, id, worker_id],
        )?;
        Ok(n > 0)
    }

    fn complete_job(&self, id: &str, worker_id: &str) -> Result<bool> {
        let n = self.conn().execute("UPDATE jobs SET status='done', lease_until=NULL WHERE id=? AND status='running' AND lease_owner=?", params![id, worker_id])?;
        Ok(n > 0)
    }

    fn fail_job(&self, id: &str, worker_id: &str, error: &str, _now: i64) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE jobs SET last_error=?, lease_until=NULL,
               status = CASE WHEN attempts >= ? THEN 'failed' ELSE 'queued' END
             WHERE id=? AND status='running' AND lease_owner=?",
            params![error, MAX_JOB_ATTEMPTS, id, worker_id],
        )?;
        Ok(n > 0)
    }

    fn fail_expired_jobs(&self, now: i64) -> Result<Vec<ExpiredJob>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let expired = {
            let mut stmt = tx.prepare("SELECT id, run_id FROM jobs WHERE status='running' AND lease_until < ? AND attempts >= ?")?;
            let rows = stmt
                .query_map(params![now, MAX_JOB_ATTEMPTS], |r| {
                    Ok(ExpiredJob {
                        job_id: r.get(0)?,
                        run_id: r.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for e in &expired {
            tx.execute("UPDATE jobs SET status='failed', lease_until=NULL, last_error=COALESCE(last_error,'lease expired too many times') WHERE id=?", [&e.job_id])?;
        }
        tx.commit()?;
        Ok(expired)
    }

    fn finish_job(
        &self,
        id: &str,
        worker_id: &str,
        result: &JobResult,
        now: i64,
    ) -> Result<FinishOutcome> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let job: Option<(String, String, Option<String>, i64)> = tx
            .query_row(
                "SELECT run_id, status, lease_owner, attempts FROM jobs WHERE id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((run_id, status, owner, attempts)) = job else {
            return Ok(FinishOutcome::NotLeased);
        };
        if status != JobStatus::Running.as_str() || owner.as_deref() != Some(worker_id) {
            return Ok(FinishOutcome::NotLeased);
        }
        let run_state: Option<String> = tx
            .query_row("SELECT state FROM runs WHERE id=?", [&run_id], |r| r.get(0))
            .optional()?;
        let open = run_state
            .as_deref()
            .and_then(RunState::parse)
            .is_some_and(|s| !s.is_terminal());
        if open {
            if let Some(f) = &result.failure {
                tx.execute(
                    "INSERT INTO failures(id,run_id,target,message,original_len,shrunk_len,bundle_hash,steps_json,created_at) VALUES (?,?,?,?,?,?,?,?,?)",
                    params![new_id("fail"), run_id, f.target, f.message, f.original_len as i64, f.shrunk_len as i64, f.bundle_hash, f.steps_json, now],
                )?;
            }
            tx.execute(
                "UPDATE runs SET state=?, steps=?, trials=?, error=?, finished_at=? WHERE id=?",
                params![
                    result.state.as_str(),
                    result.steps as i64,
                    result.trials as i64,
                    result.error,
                    now,
                    run_id
                ],
            )?;
            trials::discard_earlier_attempts(&tx, &run_id, attempts)?;
            trials::write_run_stats(&tx, &run_id, &result.stats)?;
            tx.execute(
                "INSERT INTO events(run_id,ts,kind,payload_json) VALUES (?,?,?,?)",
                params![run_id, now, "finished", result.event_payload_json],
            )?;
        }
        tx.execute(
            "UPDATE jobs SET status='done', lease_until=NULL WHERE id=?",
            [id],
        )?;
        tx.commit()?;
        Ok(if open {
            FinishOutcome::Applied
        } else {
            FinishOutcome::RunAlreadyFinal
        })
    }

    fn queued_job_count(&self) -> Result<u64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM jobs WHERE status='queued'", [], |r| {
                r.get::<_, i64>(0)
            })? as u64)
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
        let conn = self.conn();
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
        trials::count_trials(&self.conn(), run_id)
    }

    fn get_trial(&self, run_id: &str, seq: u64) -> Result<Option<TrialRecord>> {
        trials::get_trial(&self.conn(), run_id, seq)
    }

    fn list_trials(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: u32,
    ) -> Result<Vec<TrialRecord>> {
        trials::list_trials(&self.conn(), run_id, after_seq, limit)
    }

    fn list_trial_digests(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: u32,
    ) -> Result<Vec<TrialDigestRow>> {
        trials::list_trial_digests(&self.conn(), run_id, after_seq, limit)
    }

    fn get_catalog(&self, run_id: &str) -> Result<Vec<CatalogRowRecord>> {
        trials::get_catalog(&self.conn(), run_id)
    }

    fn list_run_properties(&self, run_id: &str) -> Result<Vec<RunPropertyRow>> {
        trials::list_run_properties(&self.conn(), run_id)
    }

    fn run_contributors(&self, run_id: &str) -> Result<Vec<Contributor>> {
        trials::run_contributors(&self.conn(), run_id)
    }

    fn set_worker_trusted(&self, id: &str, trusted: bool) -> Result<()> {
        trials::set_worker_trusted(&self.conn(), id, trusted)
    }

    fn worker_trusted(&self, id: &str) -> Result<Option<bool>> {
        trials::worker_trusted(&self.conn(), id)
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

    fn put_run_identity(&self, identity: &RunIdentity) -> Result<IdentityOutcome> {
        lineage::put_identity(&mut self.conn(), identity)
    }

    fn get_run_identity(&self, run_id: &str) -> Result<Option<RunIdentity>> {
        lineage::get_identity(&self.conn(), run_id)
    }

    fn create_tree(&self, new: &NewTree, now: i64) -> Result<TreeOutcome> {
        lineage::create_tree(&mut self.conn(), new, now)
    }

    fn get_tree(&self, id: &str) -> Result<Option<TreeRecord>> {
        lineage::get_tree(&self.conn(), id)
    }

    fn tree_of_run(&self, run_id: &str) -> Result<Option<String>> {
        lineage::tree_of_run(&self.conn(), run_id)
    }

    fn add_tree_node(&self, tree_id: &str, new: &NewNode, now: i64) -> Result<NodeOutcome> {
        lineage::add_node(&mut self.conn(), tree_id, new, now)
    }

    fn get_tree_node(&self, tree_id: &str, node: u64) -> Result<Option<NodeRecord>> {
        lineage::get_node(&self.conn(), tree_id, node)
    }

    fn tree_node_of_trial(&self, run_id: &str, seq: u64) -> Result<Option<NodeRecord>> {
        lineage::node_of_trial(&self.conn(), run_id, seq)
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
        query::descendants(&self.conn(), self.rule(), tree_id, node, after, limit)
    }

    fn subtree_tally(&self, tree_id: &str, node: u64) -> Result<Vec<VerdictCount>> {
        query::tally(&self.conn(), self.rule(), tree_id, node)
    }

    fn subtree_first_failing(&self, tree_id: &str, node: u64) -> Result<Option<u64>> {
        query::first_failing(&self.conn(), self.rule(), tree_id, node)
    }

    fn tree_children(
        &self,
        tree_id: &str,
        node: u64,
        after: Option<u64>,
        limit: u32,
    ) -> Result<NodePage> {
        query::children(&self.conn(), tree_id, node, after, limit)
    }

    fn tree_ancestors(&self, tree_id: &str, node: u64) -> Result<Vec<NodeRecord>> {
        query::ancestors(&self.conn(), tree_id, node)
    }

    fn add_tree_branch(&self, new: &NewBranch, now: i64) -> Result<BranchOutcome> {
        lineage::add_branch(&mut self.conn(), new, now)
    }

    fn get_tree_branch(&self, run_id: &str) -> Result<Option<BranchRecord>> {
        lineage::get_branch(&self.conn(), run_id)
    }

    fn find_tree_branch(&self, tree_id: &str, command_hash: &str) -> Result<Option<BranchRecord>> {
        lineage::find_branch(&self.conn(), tree_id, command_hash)
    }

    fn record_branch_verify(&self, run_id: &str, matched: bool) -> Result<()> {
        lineage::record_verify(&self.conn(), run_id, matched)
    }

    fn set_worker_secret_hash(&self, id: &str, hash: &str) -> Result<()> {
        trials::set_worker_secret_hash(&self.conn(), id, hash)
    }

    fn worker_secret_hash(&self, id: &str) -> Result<Option<String>> {
        trials::worker_secret_hash(&self.conn(), id)
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
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM workers", [], |r| r.get::<_, i64>(0))?
            as u64)
    }

    fn prune_workers(&self, before: i64) -> Result<u64> {
        Ok(self
            .conn()
            .execute("DELETE FROM workers WHERE last_seen < ?", [before])? as u64)
    }

    fn list_workers(&self) -> Result<Vec<WorkerRecord>> {
        let conn = self.conn();
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
    }
}
