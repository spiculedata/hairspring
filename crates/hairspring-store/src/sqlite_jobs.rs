//! The job queue on SQLite: enqueueing, leasing, heartbeats, giving a job back, failing and finishing it. The methods
//! are those of the `Store` implementation, moved here to keep `sqlite.rs` short; the trait methods call them.
use crate::sqlite::job_from_row;
use crate::sqlite_trials as trials;
use crate::sqlite_tx::write_tx;
use crate::types::*;
use crate::{new_id, SqliteStore, MAX_JOB_ATTEMPTS};
use rusqlite::{params, OptionalExtension};

impl SqliteStore {
    pub(crate) fn jobs_enqueue_job(
        &self,
        run_id: &str,
        kind: &str,
        payload_json: &str,
        now: i64,
    ) -> Result<Job> {
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

    pub(crate) fn jobs_get_job(&self, id: &str) -> Result<Option<Job>> {
        Ok(self
            .conn()
            .query_row("SELECT * FROM jobs WHERE id=?", [id], job_from_row)
            .optional()?)
    }

    pub(crate) fn jobs_lease_job(
        &self,
        worker_id: &str,
        lease_secs: u64,
        now: i64,
    ) -> Result<Option<Job>> {
        let mut conn = self.conn();
        let tx = write_tx(&mut conn)?;
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

    pub(crate) fn jobs_heartbeat_job(
        &self,
        id: &str,
        worker_id: &str,
        lease_secs: u64,
        now: i64,
    ) -> Result<bool> {
        let until = now.saturating_add((lease_secs as i64).saturating_mul(1000));
        let n = self.conn().execute(
            "UPDATE jobs SET lease_until=? WHERE id=? AND status='running' AND lease_owner=?",
            params![until, id, worker_id],
        )?;
        Ok(n > 0)
    }

    pub(crate) fn jobs_complete_job(&self, id: &str, worker_id: &str) -> Result<bool> {
        let n = self.conn().execute("UPDATE jobs SET status='done', lease_until=NULL WHERE id=? AND status='running' AND lease_owner=?", params![id, worker_id])?;
        Ok(n > 0)
    }

    pub(crate) fn jobs_fail_job(
        &self,
        id: &str,
        worker_id: &str,
        error: &str,
        _now: i64,
    ) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE jobs SET last_error=?, lease_until=NULL,
               status = CASE WHEN attempts >= ? THEN 'failed' ELSE 'queued' END
             WHERE id=? AND status='running' AND lease_owner=?",
            params![error, MAX_JOB_ATTEMPTS, id, worker_id],
        )?;
        Ok(n > 0)
    }

    pub(crate) fn jobs_release_job(&self, id: &str, worker_id: &str) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE jobs SET status='queued', lease_owner=NULL, lease_until=NULL, attempts=attempts-1
             WHERE id=? AND status='running' AND lease_owner=? AND attempts > 0",
            params![id, worker_id],
        )?;
        Ok(n > 0)
    }

    pub(crate) fn jobs_fail_expired_jobs(&self, now: i64) -> Result<Vec<ExpiredJob>> {
        let mut conn = self.conn();
        let tx = write_tx(&mut conn)?;
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

    pub(crate) fn jobs_finish_job(
        &self,
        id: &str,
        worker_id: &str,
        result: &JobResult,
        now: i64,
    ) -> Result<FinishOutcome> {
        let mut conn = self.conn();
        let tx = write_tx(&mut conn)?;
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
            for f in result.failure.iter().chain(&result.more_failures) {
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

    pub(crate) fn jobs_queued_job_count(&self) -> Result<u64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM jobs WHERE status='queued'", [], |r| {
                r.get::<_, i64>(0)
            })? as u64)
    }
}
