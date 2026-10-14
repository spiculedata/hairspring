use crate::types::*;
use crate::{new_id, sha256_hex, Store, MAX_JOB_ATTEMPTS};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
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

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Backend(e.to_string())
    }
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
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
        conn.pragma_update(None, "journal_mode", "WAL")?;
        SqliteStore::init(conn)
    }

    pub fn open_in_memory() -> Result<SqliteStore> {
        SqliteStore::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<SqliteStore> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else if version != SCHEMA_VERSION {
            return Err(StoreError::Backend(format!(
                "database schema version {version} is not supported (expected {SCHEMA_VERSION})"
            )));
        }
        Ok(SqliteStore {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A poisoned mutex only means another thread panicked mid-query; SQLite state is still consistent.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
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
    ) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE runs SET state=?, steps=?, trials=?, error=?, finished_at=? WHERE id=? AND state NOT IN ('cancelled')",
            params![state.as_str(), steps as i64, trials as i64, error, now, id],
        )?;
        expect_changed(n, "run", id)
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
        tx.execute(
            "UPDATE jobs SET status='failed', last_error=COALESCE(last_error,'lease expired too many times') WHERE status='running' AND lease_until < ? AND attempts >= ?",
            params![now, MAX_JOB_ATTEMPTS],
        )?;
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
        })
    }

    fn touch_worker(&self, id: &str, now: i64) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE workers SET last_seen=? WHERE id=?",
            params![now, id],
        )?;
        expect_changed(n, "worker", id)
    }

    fn list_workers(&self) -> Result<Vec<WorkerRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id,name,caps_json,registered_at,last_seen FROM workers ORDER BY registered_at, rowid")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(WorkerRecord {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    caps_json: r.get(2)?,
                    registered_at: r.get(3)?,
                    last_seen: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
