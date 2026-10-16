//! Job lifecycle: leasing, progress, heartbeats, completion and local execution.
use crate::completion::hit_time_or_step_limit;
use crate::sanitize;
use crate::types::{JobPayload, LeasedJob, RunDetail, StartRun};
use crate::{Result, Service, ServiceError};
use hairspring_engine::backend::Progress;
use hairspring_engine::config::Budget;
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::report::{RunOutcome, RunStatus};
use hairspring_store::{
    now_ms, FinishOutcome, JobResult, JobStatus, NewFailure, RunState, RunStats,
};
use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const HEARTBEAT_DIVISOR: u64 = 3;
const MIN_HEARTBEAT: Duration = Duration::from_millis(100);
const HEARTBEAT_TICK: Duration = Duration::from_millis(25);
const SYNC_POLL: Duration = Duration::from_millis(100);
const SYNC_MAX_WAIT: Duration = Duration::from_secs(3600);
const PROGRESS_FORGET_AFTER: Duration = Duration::from_secs(60);
const MAX_TRACKED_RUNS: usize = 10_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Abort {
    None,
    Stop,
    Cancelled,
}

fn finished_payload(state: RunState, outcome: Option<&RunOutcome>) -> String {
    serde_json::json!({
        "state": state.as_str(),
        "inconclusive": outcome.map_or(0, |o| o.inconclusive),
        "distinct_schedules": outcome.map_or(0, |o| o.distinct_schedules),
        "work": outcome.map_or(0, |o| o.work),
    })
    .to_string()
}

impl Service {
    /// Leases the next runnable job and marks its run as started. Jobs that ran out of
    /// attempts through lease expiry are finalised here so their runs never hang.
    pub fn lease(&self, worker_id: &str) -> Result<Option<LeasedJob>> {
        for expired in self.store.fail_expired_jobs(now_ms())? {
            self.finalise_run_as_error(
                &expired.run_id,
                "the worker's lease expired too many times",
            )?;
        }
        loop {
            let Some(job) = self
                .store
                .lease_job(worker_id, self.cfg.lease_secs, now_ms())?
            else {
                return Ok(None);
            };
            let payload: JobPayload = match serde_json::from_str(&job.payload_json) {
                Ok(p) => p,
                Err(e) => {
                    self.fail(&job.id, worker_id, &format!("unreadable job payload: {e}"))?;
                    continue;
                }
            };
            match self.store.get_run(&job.run_id)? {
                Some(run) if run.state.is_terminal() => {
                    self.store.complete_job(&job.id, worker_id)?;
                    continue;
                }
                None => {
                    self.fail(&job.id, worker_id, "run no longer exists")?;
                    continue;
                }
                Some(_) => {}
            }
            let now = now_ms();
            self.store.mark_running(&job.run_id, now)?;
            self.store.add_event(
                &job.run_id,
                "started",
                &serde_json::json!({ "worker": worker_id }).to_string(),
                now,
            )?;
            return Ok(Some(LeasedJob {
                job_id: job.id,
                run_id: job.run_id,
                target: payload.target,
                seed: payload.seed,
                budget: payload.budget,
            }));
        }
    }

    pub fn heartbeat(&self, job_id: &str, worker_id: &str) -> Result<bool> {
        Ok(self
            .store
            .heartbeat_job(job_id, worker_id, self.cfg.lease_secs, now_ms())?)
    }

    fn should_store_progress_event(&self, run_id: &str) -> bool {
        let mut seen = self.progress_seen.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if seen.len() > MAX_TRACKED_RUNS {
            seen.retain(|_, t| now.duration_since(*t) < PROGRESS_FORGET_AFTER);
        }
        match seen.get(run_id) {
            Some(t) if now.duration_since(*t) < self.cfg.progress_event_interval => false,
            _ => {
                seen.insert(run_id.to_string(), now);
                true
            }
        }
    }

    pub fn report_progress(&self, run_id: &str, p: &Progress) -> Result<()> {
        self.store.update_progress(run_id, p.steps, p.trials)?;
        if self.should_store_progress_event(run_id) {
            let payload =
                serde_json::json!({ "steps": p.steps, "trials": p.trials, "message": p.message });
            self.store
                .add_event(run_id, "progress", &payload.to_string(), now_ms())?;
        }
        Ok(())
    }

    pub(crate) fn owned_running_job(
        &self,
        job_id: &str,
        worker_id: &str,
    ) -> Result<hairspring_store::Job> {
        let job = self
            .store
            .get_job(job_id)?
            .ok_or_else(|| ServiceError::NotFound(format!("job {job_id}")))?;
        if job.status != JobStatus::Running || job.lease_owner.as_deref() != Some(worker_id) {
            return Err(ServiceError::InvalidRequest(
                "job is not leased by this worker".to_string(),
            ));
        }
        Ok(job)
    }

    /// Progress from a (possibly remote) worker; only the current lease holder may report.
    /// Returns whether the worker should keep going (false once the run was cancelled).
    pub fn report_job_progress(&self, job_id: &str, worker_id: &str, p: &Progress) -> Result<bool> {
        let job = self.owned_running_job(job_id, worker_id)?;
        self.report_progress(&job.run_id, p)?;
        self.heartbeat(job_id, worker_id)?;
        let cancelled = self
            .store
            .get_run(&job.run_id)?
            .is_some_and(|r| r.state == RunState::Cancelled);
        Ok(!cancelled)
    }

    /// Records the outcome of a leased job (used by local and remote workers alike).
    /// The result is validated and sanitised, then written atomically.
    pub fn complete(&self, job_id: &str, worker_id: &str, outcome: &RunOutcome) -> Result<()> {
        let job = self.owned_running_job(job_id, worker_id)?;
        let run = self
            .store
            .get_run(&job.run_id)?
            .ok_or_else(|| ServiceError::NotFound(format!("run {}", job.run_id)))?;
        let clean = sanitize::outcome(outcome, &run.target)?;
        let reported = self
            .ingest_local_log(job_id, worker_id, outcome)?
            .or(clean.trials_reported);
        let state = match clean.status {
            RunStatus::Passed => RunState::Passed,
            RunStatus::Failed => RunState::Failed,
            RunStatus::Error => RunState::Error,
        };
        let failure = match &clean.failure {
            Some(f) => {
                let bundle_hash = self.store.put_blob(f.bundle.as_bytes())?;
                let steps_json = serde_json::to_string(&f.steps)
                    .map_err(|e| ServiceError::Internal(e.to_string()))?;
                Some(NewFailure {
                    run_id: job.run_id.clone(),
                    target: f.target.clone(),
                    message: f.message.clone(),
                    original_len: f.original_len,
                    shrunk_len: f.shrunk_len,
                    bundle_hash,
                    steps_json,
                })
            }
            None => None,
        };
        let budget = serde_json::from_str::<JobPayload>(&job.payload_json)
            .ok()
            .and_then(|p| p.budget);
        let stats = self.run_stats(&job.run_id, job.attempts, &clean, reported, budget)?;
        let event_payload_json = finished_payload(state, Some(&clean));
        let result = JobResult {
            state,
            steps: clean.steps_executed,
            trials: clean.trials,
            error: clean.error,
            failure,
            event_payload_json,
            stats,
        };
        match self
            .store
            .finish_job(job_id, worker_id, &result, now_ms())?
        {
            FinishOutcome::NotLeased => Err(ServiceError::InvalidRequest(
                "job is not leased by this worker".to_string(),
            )),
            FinishOutcome::Applied | FinishOutcome::RunAlreadyFinal => {
                self.forget_progress(&job.run_id);
                Ok(())
            }
        }
    }

    /// Stores the trial records an in-process worker handed over inside its outcome, batch by batch, and returns how
    /// many it handed over. A remote worker sends its records through the ingest endpoint instead.
    fn ingest_local_log(
        &self,
        job_id: &str,
        worker_id: &str,
        outcome: &RunOutcome,
    ) -> Result<Option<u64>> {
        if outcome.trial_log.is_empty() {
            return Ok(None);
        }
        for batch in crate::ingest::batches(&outcome.trial_log) {
            self.ingest_trials(job_id, worker_id, batch)?;
        }
        Ok(Some(outcome.trial_log.len() as u64))
    }

    /// The figures to store with a finished run. The statistics are folded here from the stored digests; whatever the
    /// worker put in its outcome is not read. A run whose rows do not match what the worker says it ran, or that did
    /// not run to the end of its budget, makes no must-hit claim.
    fn run_stats(
        &self,
        run_id: &str,
        attempt: u32,
        clean: &RunOutcome,
        reported: Option<u64>,
        budget: Option<Budget>,
    ) -> Result<RunStats> {
        // Rows an earlier attempt left belong to a run that is over; they are removed when this job finishes.
        let stale = self
            .store
            .get_run(run_id)?
            .is_some_and(|r| r.ingest_attempt != 0 && r.ingest_attempt < u64::from(attempt));
        let held = if stale {
            0
        } else {
            self.store.count_trials(run_id)?
        };
        let counts_match = match reported {
            None => held == 0,
            Some(n) => n == held && (clean.status == RunStatus::Error || clean.trials == held),
        };
        // A run that passed without holding as many trials as it was asked for ended early. If it ended on its
        // wall-clock or step limit, the server's own clock and the step count say so, nothing is missing, and its
        // result stands for the trials it ran. Otherwise whoever called `complete` stopped it, and the trials it never
        // ran cannot back a claim.
        let uses_rows = held > 0 || reported.is_some();
        let short = clean.status == RunStatus::Passed
            && uses_rows
            && budget.is_some_and(|b| held < b.max_trials);
        let limited = short
            && budget.is_some_and(|b| {
                let started = self
                    .store
                    .get_run(run_id)
                    .ok()
                    .flatten()
                    .and_then(|r| r.started_at);
                hit_time_or_step_limit(&b, started, now_ms(), clean.steps_executed, attempt)
            });
        let mut incomplete = !counts_match || (short && !limited);
        let (properties, property_summary_json) = if held == 0 {
            (Vec::new(), None)
        } else {
            match self.fold_run(run_id) {
                Ok(folded) => {
                    let withhold = incomplete || clean.status != RunStatus::Passed;
                    self.property_rows(&folded, withhold)
                }
                Err(e) => {
                    eprintln!("hairspring: cannot fold the trials of {run_id}: {e}");
                    incomplete = true;
                    (Vec::new(), None)
                }
            }
        };
        Ok(RunStats {
            inconclusive: clean.inconclusive,
            distinct_schedules: clean.distinct_schedules,
            work: clean.work,
            stopped_at_first_failure: clean.status == RunStatus::Failed,
            ingest_incomplete: incomplete,
            property_summary_json,
            properties,
        })
    }

    fn forget_progress(&self, run_id: &str) {
        self.progress_seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(run_id);
    }

    fn finalise_run_as_error(&self, run_id: &str, reason: &str) -> Result<()> {
        let now = now_ms();
        if self
            .store
            .finish_run(run_id, RunState::Error, 0, 0, Some(reason), now)?
        {
            self.store.add_event(
                run_id,
                "finished",
                &finished_payload(RunState::Error, None),
                now,
            )?;
        }
        Ok(())
    }

    /// Records a transient worker failure; the job is retried until attempts run out,
    /// at which point its run is finalised as an error.
    pub fn fail(&self, job_id: &str, worker_id: &str, error: &str) -> Result<()> {
        let now = now_ms();
        if !self.store.fail_job(job_id, worker_id, error, now)? {
            return Ok(()); // not the lease holder (stale report): nothing to change
        }
        if let Some(job) = self.store.get_job(job_id)? {
            if job.status == JobStatus::Failed {
                self.finalise_run_as_error(&job.run_id, error)?;
            }
        }
        Ok(())
    }

    /// Leases and executes one job locally. Returns false when the queue is empty.
    pub fn execute_next(&self, engine: &Engine, worker_id: &str) -> Result<bool> {
        self.execute_next_until(engine, worker_id, &AtomicBool::new(false))
    }

    /// Like [`Service::execute_next`], but stops the job (and requeues it) when `stop` becomes true.
    /// A heartbeat keeps the lease alive while the run is silent; cancelling the run stops it too.
    pub fn execute_next_until(
        &self,
        engine: &Engine,
        worker_id: &str,
        stop: &AtomicBool,
    ) -> Result<bool> {
        let Some(job) = self.lease(worker_id)? else {
            return Ok(false);
        };
        let req = RunRequest {
            target: job.target.clone(),
            seed: job.seed,
            budget: job.budget,
        };
        let abort = Cell::new(Abort::None);
        let done = AtomicBool::new(false);
        let heartbeat_every = Duration::from_millis(
            (self.cfg.lease_secs * 1000 / HEARTBEAT_DIVISOR).max(MIN_HEARTBEAT.as_millis() as u64),
        );

        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut last = Instant::now();
                while !done.load(Ordering::SeqCst) {
                    std::thread::sleep(HEARTBEAT_TICK);
                    if last.elapsed() >= heartbeat_every {
                        // Best effort: a missed beat is retried on the next tick.
                        let _ = self.heartbeat(&job.job_id, worker_id);
                        last = Instant::now();
                    }
                }
            });
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                engine.run(&req, &mut |p| {
                    let _ = self.report_progress(&job.run_id, &p);
                    let _ = self.heartbeat(&job.job_id, worker_id);
                    if stop.load(Ordering::SeqCst) {
                        abort.set(Abort::Stop);
                        return false;
                    }
                    if self
                        .store
                        .get_run(&job.run_id)
                        .ok()
                        .flatten()
                        .is_some_and(|r| r.state == RunState::Cancelled)
                    {
                        abort.set(Abort::Cancelled);
                        return false;
                    }
                    true
                })
            }));
            done.store(true, Ordering::SeqCst);
            outcome
        });

        match (result, abort.get()) {
            (Ok(_), Abort::Stop) => self.fail(&job.job_id, worker_id, "interrupted by shutdown")?,
            (Ok(Ok(outcome)), _) => self.complete(&job.job_id, worker_id, &outcome)?,
            (Ok(Err(e)), _) => {
                self.complete(&job.job_id, worker_id, &RunOutcome::error(e.to_string()))?
            }
            (Err(_), _) => self.fail(
                &job.job_id,
                worker_id,
                "worker panicked while running the target",
            )?,
        }
        Ok(true)
    }

    /// Starts a run and drives the queue until that run is finished (CLI / local mode).
    pub fn run_sync(&self, engine: &Engine, req: &StartRun) -> Result<RunDetail> {
        let run = self.start_run(engine, req)?;
        let worker = Service::local_worker_id();
        let deadline = Instant::now() + SYNC_MAX_WAIT;
        loop {
            let detail = self
                .get_run(&run.id)?
                .ok_or_else(|| ServiceError::NotFound(run.id.clone()))?;
            if detail.run.state.is_terminal() {
                return Ok(detail);
            }
            // If nothing is left to execute here, another process may hold our job; wait for it.
            if !self.execute_next(engine, &worker)? {
                if Instant::now() >= deadline {
                    return Err(ServiceError::Internal(
                        "timed out waiting for the run to finish".to_string(),
                    ));
                }
                std::thread::sleep(SYNC_POLL);
            }
        }
    }
}
