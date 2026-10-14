//! PostgreSQL implementation of [`Store`] for multi-worker deployments.
//!
//! Connections are plain TCP (`NoTls`): run Postgres on a private network or behind a TLS proxy.
//! Leasing uses `FOR UPDATE SKIP LOCKED`, so any number of workers can pull jobs concurrently.
use crate::types::*;
use crate::{new_id, sha256_hex, Store, MAX_JOB_ATTEMPTS};
use postgres::{Client, Config, NoTls, Row};
use r2d2_postgres::{r2d2, PostgresConnectionManager};

type Pool = r2d2::Pool<PostgresConnectionManager<NoTls>>;

const SCHEMA_VERSION: i32 = 1;
/// Serialises migrations across processes starting at the same time.
const MIGRATION_LOCK: i64 = 727_274;
const MAX_SCHEMA_NAME: usize = 63;

const SCHEMA: &str = "
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

impl From<postgres::Error> for StoreError {
    fn from(e: postgres::Error) -> Self {
        StoreError::Backend(e.to_string())
    }
}

impl From<r2d2::Error> for StoreError {
    fn from(e: r2d2::Error) -> Self {
        StoreError::Backend(format!("connection pool: {e}"))
    }
}

pub struct PgStore {
    pool: Pool,
}

fn valid_schema(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SCHEMA_NAME
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn run_from_row(r: &Row) -> RunRecord {
    let state: String = r.get("state");
    RunRecord {
        id: r.get("id"),
        target: r.get("target"),
        backend: r.get("backend"),
        state: RunState::parse(&state).unwrap_or(RunState::Error),
        seed: r.get::<_, i64>("seed") as u64,
        config_json: r.get("config_json"),
        created_at: r.get("created_at"),
        started_at: r.get("started_at"),
        finished_at: r.get("finished_at"),
        steps: r.get::<_, i64>("steps") as u64,
        trials: r.get::<_, i64>("trials") as u64,
        error: r.get("error"),
    }
}

fn failure_from_row(r: &Row) -> FailureRecord {
    FailureRecord {
        id: r.get("id"),
        run_id: r.get("run_id"),
        target: r.get("target"),
        message: r.get("message"),
        original_len: r.get::<_, i64>("original_len") as usize,
        shrunk_len: r.get::<_, i64>("shrunk_len") as usize,
        bundle_hash: r.get("bundle_hash"),
        steps_json: r.get("steps_json"),
        created_at: r.get("created_at"),
    }
}

fn job_from_row(r: &Row) -> Job {
    let status: String = r.get("status");
    Job {
        id: r.get("id"),
        run_id: r.get("run_id"),
        kind: r.get("kind"),
        payload_json: r.get("payload_json"),
        status: JobStatus::parse(&status).unwrap_or(JobStatus::Failed),
        lease_owner: r.get("lease_owner"),
        lease_until: r.get("lease_until"),
        attempts: r.get::<_, i32>("attempts") as u32,
        last_error: r.get("last_error"),
        created_at: r.get("created_at"),
    }
}

impl PgStore {
    /// Connects to the `public` schema.
    pub fn connect(url: &str, pool_size: u32) -> Result<PgStore> {
        PgStore::connect_in_schema(url, "public", pool_size)
    }

    /// Connects, creating `schema` if needed and migrating it. The schema name is validated and quoted.
    pub fn connect_in_schema(url: &str, schema: &str, pool_size: u32) -> Result<PgStore> {
        if !valid_schema(schema) {
            return Err(StoreError::Backend("invalid schema name".to_string()));
        }
        // Parse errors are reported without echoing the URL (it contains the password).
        let mut cfg: Config = url
            .parse()
            .map_err(|_| StoreError::Backend("invalid database URL".to_string()))?;
        {
            let mut admin = cfg.connect(NoTls)?;
            // `CREATE SCHEMA IF NOT EXISTS` itself races when several processes start together.
            admin.execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK])?;
            let created = admin.batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\""));
            admin.execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK])?;
            created?;
        }
        cfg.options(&format!("-c search_path=\"{schema}\""));
        let pool = r2d2::Pool::builder()
            .max_size(pool_size.max(1))
            .build(PostgresConnectionManager::new(cfg, NoTls))?;
        let store = PgStore { pool };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let mut c = self.pool.get()?;
        c.execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK])?;
        let result = (|| -> Result<()> {
            c.batch_execute(
                "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)",
            )?;
            let current: Option<i32> = c
                .query_opt("SELECT version FROM schema_version LIMIT 1", &[])?
                .map(|r| r.get(0));
            match current {
                None => {
                    let mut tx = c.transaction()?;
                    tx.batch_execute(SCHEMA)?;
                    tx.execute(
                        "INSERT INTO schema_version(version) VALUES ($1)",
                        &[&SCHEMA_VERSION],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
                Some(v) if v == SCHEMA_VERSION => Ok(()),
                Some(v) => Err(StoreError::Backend(format!(
                    "database schema version {v} is not supported (expected {SCHEMA_VERSION})"
                ))),
            }
        })();
        c.execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK])?;
        result
    }

    fn client(&self) -> Result<r2d2::PooledConnection<PostgresConnectionManager<NoTls>>> {
        Ok(self.pool.get()?)
    }
}

fn expect_changed(changed: u64, what: &str, id: &str) -> Result<()> {
    if changed == 0 {
        Err(StoreError::NotFound(format!("{what} {id}")))
    } else {
        Ok(())
    }
}

fn query_all<T>(
    c: &mut Client,
    sql: &str,
    params: &[&(dyn postgres::types::ToSql + Sync)],
    map: fn(&Row) -> T,
) -> Result<Vec<T>> {
    Ok(c.query(sql, params)?.iter().map(map).collect())
}

impl Store for PgStore {
    fn create_run(&self, new: &NewRun, now: i64) -> Result<RunRecord> {
        let id = new_id("run");
        let mut c = self.client()?;
        let row = c.query_one(
            "INSERT INTO runs(id,target,backend,state,seed,config_json,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING *",
            &[&id, &new.target, &new.backend, &RunState::Queued.as_str(), &(new.seed as i64), &new.config_json, &now],
        )?;
        Ok(run_from_row(&row))
    }

    fn get_run(&self, id: &str) -> Result<Option<RunRecord>> {
        Ok(self
            .client()?
            .query_opt("SELECT * FROM runs WHERE id=$1", &[&id])?
            .map(|r| run_from_row(&r)))
    }

    fn list_runs(&self, limit: u32) -> Result<Vec<RunRecord>> {
        query_all(
            &mut *self.client()?,
            "SELECT * FROM runs ORDER BY created_at DESC, seq DESC LIMIT $1",
            &[&i64::from(limit)],
            run_from_row,
        )
    }

    fn mark_running(&self, id: &str, now: i64) -> Result<()> {
        let n = self.client()?.execute(
            "UPDATE runs SET state=$1, started_at=COALESCE(started_at,$2) WHERE id=$3 AND state IN ('queued','running')",
            &[&RunState::Running.as_str(), &now, &id],
        )?;
        expect_changed(n, "run", id)
    }

    fn update_progress(&self, id: &str, steps: u64, trials: u64) -> Result<()> {
        let n = self.client()?.execute(
            "UPDATE runs SET steps=$1, trials=$2 WHERE id=$3",
            &[&(steps as i64), &(trials as i64), &id],
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
        let mut c = self.client()?;
        let n = c.execute(
            "UPDATE runs SET state=$1, steps=$2, trials=$3, error=$4, finished_at=$5 WHERE id=$6 AND state IN ('queued','running')",
            &[&state.as_str(), &(steps as i64), &(trials as i64), &error, &now, &id],
        )?;
        if n == 0
            && c.query_opt("SELECT 1 FROM runs WHERE id=$1", &[&id])?
                .is_none()
        {
            return Err(StoreError::NotFound(format!("run {id}")));
        }
        Ok(n > 0)
    }

    fn cancel_run(&self, id: &str, now: i64) -> Result<bool> {
        let mut c = self.client()?;
        let mut tx = c.transaction()?;
        let n = tx.execute("UPDATE runs SET state='cancelled', finished_at=$1 WHERE id=$2 AND state IN ('queued','running')", &[&now, &id])?;
        if n > 0 {
            tx.execute("UPDATE jobs SET status='failed', last_error='cancelled' WHERE run_id=$1 AND status='queued'", &[&id])?;
        }
        tx.commit()?;
        Ok(n > 0)
    }

    fn add_failure(&self, new: &NewFailure, now: i64) -> Result<FailureRecord> {
        let id = new_id("fail");
        let row = self.client()?.query_one(
            "INSERT INTO failures(id,run_id,target,message,original_len,shrunk_len,bundle_hash,steps_json,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING *",
            &[&id, &new.run_id, &new.target, &new.message, &(new.original_len as i64), &(new.shrunk_len as i64), &new.bundle_hash, &new.steps_json, &now],
        )?;
        Ok(failure_from_row(&row))
    }

    fn get_failure(&self, id: &str) -> Result<Option<FailureRecord>> {
        Ok(self
            .client()?
            .query_opt("SELECT * FROM failures WHERE id=$1", &[&id])?
            .map(|r| failure_from_row(&r)))
    }

    fn list_failures(&self, run_id: &str) -> Result<Vec<FailureRecord>> {
        query_all(
            &mut *self.client()?,
            "SELECT * FROM failures WHERE run_id=$1 ORDER BY created_at, seq",
            &[&run_id],
            failure_from_row,
        )
    }

    fn list_recent_failures(&self, limit: u32) -> Result<Vec<FailureRecord>> {
        query_all(
            &mut *self.client()?,
            "SELECT * FROM failures ORDER BY created_at DESC, seq DESC LIMIT $1",
            &[&i64::from(limit)],
            failure_from_row,
        )
    }

    fn put_blob(&self, data: &[u8]) -> Result<String> {
        let hash = sha256_hex(data);
        self.client()?.execute(
            "INSERT INTO blobs(hash,data) VALUES ($1,$2) ON CONFLICT DO NOTHING",
            &[&hash, &data],
        )?;
        Ok(hash)
    }

    fn get_blob(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .client()?
            .query_opt("SELECT data FROM blobs WHERE hash=$1", &[&hash])?
            .map(|r| r.get(0)))
    }

    fn enqueue_job(&self, run_id: &str, kind: &str, payload_json: &str, now: i64) -> Result<Job> {
        let id = new_id("job");
        let row = self.client()?.query_one(
            "INSERT INTO jobs(id,run_id,kind,payload_json,status,created_at) VALUES ($1,$2,$3,$4,$5,$6) RETURNING *",
            &[&id, &run_id, &kind, &payload_json, &JobStatus::Queued.as_str(), &now],
        )?;
        Ok(job_from_row(&row))
    }

    fn get_job(&self, id: &str) -> Result<Option<Job>> {
        Ok(self
            .client()?
            .query_opt("SELECT * FROM jobs WHERE id=$1", &[&id])?
            .map(|r| job_from_row(&r)))
    }

    fn lease_job(&self, worker_id: &str, lease_secs: u64, now: i64) -> Result<Option<Job>> {
        let mut c = self.client()?;
        let mut tx = c.transaction()?;
        let max_attempts = MAX_JOB_ATTEMPTS as i32;
        // SKIP LOCKED: concurrent workers each get a different job instead of queueing behind one another.
        let Some(row) = tx.query_opt(
            "SELECT id FROM jobs WHERE (status='queued' OR (status='running' AND lease_until < $1)) AND attempts < $2
             ORDER BY created_at, seq LIMIT 1 FOR UPDATE SKIP LOCKED",
            &[&now, &max_attempts],
        )?
        else {
            tx.commit()?;
            return Ok(None);
        };
        let id: String = row.get(0);
        let until = now.saturating_add((lease_secs as i64).saturating_mul(1000));
        let job = tx.query_one(
            "UPDATE jobs SET status='running', lease_owner=$1, lease_until=$2, attempts=attempts+1 WHERE id=$3 RETURNING *",
            &[&worker_id, &until, &id],
        )?;
        tx.commit()?;
        Ok(Some(job_from_row(&job)))
    }

    fn heartbeat_job(&self, id: &str, worker_id: &str, lease_secs: u64, now: i64) -> Result<bool> {
        let until = now.saturating_add((lease_secs as i64).saturating_mul(1000));
        let n = self.client()?.execute(
            "UPDATE jobs SET lease_until=$1 WHERE id=$2 AND status='running' AND lease_owner=$3",
            &[&until, &id, &worker_id],
        )?;
        Ok(n > 0)
    }

    fn complete_job(&self, id: &str, worker_id: &str) -> Result<bool> {
        let n = self.client()?.execute("UPDATE jobs SET status='done', lease_until=NULL WHERE id=$1 AND status='running' AND lease_owner=$2", &[&id, &worker_id])?;
        Ok(n > 0)
    }

    fn fail_expired_jobs(&self, now: i64) -> Result<Vec<ExpiredJob>> {
        let rows = self.client()?.query(
            "UPDATE jobs SET status='failed', lease_until=NULL, last_error=COALESCE(last_error,'lease expired too many times')
             WHERE status='running' AND lease_until < $1 AND attempts >= $2 RETURNING id, run_id",
            &[&now, &(MAX_JOB_ATTEMPTS as i32)],
        )?;
        Ok(rows
            .iter()
            .map(|r| ExpiredJob {
                job_id: r.get(0),
                run_id: r.get(1),
            })
            .collect())
    }

    fn finish_job(
        &self,
        id: &str,
        worker_id: &str,
        result: &JobResult,
        now: i64,
    ) -> Result<FinishOutcome> {
        let mut c = self.client()?;
        let mut tx = c.transaction()?;
        let Some(job) = tx.query_opt(
            "SELECT run_id, status, lease_owner FROM jobs WHERE id=$1 FOR UPDATE",
            &[&id],
        )?
        else {
            return Ok(FinishOutcome::NotLeased);
        };
        let (run_id, status, owner): (String, String, Option<String>) =
            (job.get(0), job.get(1), job.get(2));
        if status != JobStatus::Running.as_str() || owner.as_deref() != Some(worker_id) {
            return Ok(FinishOutcome::NotLeased);
        }
        let run_state: Option<String> = tx
            .query_opt("SELECT state FROM runs WHERE id=$1 FOR UPDATE", &[&run_id])?
            .map(|r| r.get(0));
        let open = run_state
            .as_deref()
            .and_then(RunState::parse)
            .is_some_and(|s| !s.is_terminal());
        if open {
            if let Some(f) = &result.failure {
                tx.execute(
                    "INSERT INTO failures(id,run_id,target,message,original_len,shrunk_len,bundle_hash,steps_json,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                    &[&new_id("fail"), &run_id, &f.target, &f.message, &(f.original_len as i64), &(f.shrunk_len as i64), &f.bundle_hash, &f.steps_json, &now],
                )?;
            }
            tx.execute(
                "UPDATE runs SET state=$1, steps=$2, trials=$3, error=$4, finished_at=$5 WHERE id=$6",
                &[&result.state.as_str(), &(result.steps as i64), &(result.trials as i64), &result.error, &now, &run_id],
            )?;
            tx.execute(
                "INSERT INTO events(run_id,ts,kind,payload_json) VALUES ($1,$2,'finished',$3)",
                &[&run_id, &now, &result.event_payload_json],
            )?;
        }
        tx.execute(
            "UPDATE jobs SET status='done', lease_until=NULL WHERE id=$1",
            &[&id],
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
            .client()?
            .query_one("SELECT COUNT(*) FROM jobs WHERE status='queued'", &[])?
            .get::<_, i64>(0) as u64)
    }

    fn fail_job(&self, id: &str, worker_id: &str, error: &str, _now: i64) -> Result<bool> {
        let n = self.client()?.execute(
            "UPDATE jobs SET last_error=$1, lease_until=NULL, status = CASE WHEN attempts >= $2 THEN 'failed' ELSE 'queued' END
             WHERE id=$3 AND status='running' AND lease_owner=$4",
            &[&error, &(MAX_JOB_ATTEMPTS as i32), &id, &worker_id],
        )?;
        Ok(n > 0)
    }

    fn add_event(&self, run_id: &str, kind: &str, payload_json: &str, now: i64) -> Result<i64> {
        Ok(self
            .client()?
            .query_one(
                "INSERT INTO events(run_id,ts,kind,payload_json) VALUES ($1,$2,$3,$4) RETURNING id",
                &[&run_id, &now, &kind, &payload_json],
            )?
            .get(0))
    }

    fn list_events(&self, run_id: &str, after_id: i64, limit: u32) -> Result<Vec<EventRecord>> {
        let rows = self.client()?.query("SELECT id,run_id,ts,kind,payload_json FROM events WHERE run_id=$1 AND id>$2 ORDER BY id LIMIT $3", &[&run_id, &after_id, &i64::from(limit)])?;
        Ok(rows
            .iter()
            .map(|r| EventRecord {
                id: r.get(0),
                run_id: r.get(1),
                ts: r.get(2),
                kind: r.get(3),
                payload_json: r.get(4),
            })
            .collect())
    }

    fn register_worker(&self, name: &str, caps_json: &str, now: i64) -> Result<WorkerRecord> {
        let id = new_id("wrk");
        self.client()?.execute("INSERT INTO workers(id,name,caps_json,registered_at,last_seen) VALUES ($1,$2,$3,$4,$4)", &[&id, &name, &caps_json, &now])?;
        Ok(WorkerRecord {
            id,
            name: name.to_string(),
            caps_json: caps_json.to_string(),
            registered_at: now,
            last_seen: now,
        })
    }

    fn touch_worker(&self, id: &str, now: i64) -> Result<()> {
        let n = self
            .client()?
            .execute("UPDATE workers SET last_seen=$1 WHERE id=$2", &[&now, &id])?;
        expect_changed(n, "worker", id)
    }

    fn list_workers(&self) -> Result<Vec<WorkerRecord>> {
        let rows = self.client()?.query("SELECT id,name,caps_json,registered_at,last_seen FROM workers ORDER BY registered_at, seq", &[])?;
        Ok(rows
            .iter()
            .map(|r| WorkerRecord {
                id: r.get(0),
                name: r.get(1),
                caps_json: r.get(2),
                registered_at: r.get(3),
                last_seen: r.get(4),
            })
            .collect())
    }

    fn worker_count(&self) -> Result<u64> {
        Ok(self
            .client()?
            .query_one("SELECT COUNT(*) FROM workers", &[])?
            .get::<_, i64>(0) as u64)
    }

    fn prune_workers(&self, before: i64) -> Result<u64> {
        Ok(self
            .client()?
            .execute("DELETE FROM workers WHERE last_seen < $1", &[&before])?)
    }
}
