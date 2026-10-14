//! Service layer: the single place that turns "start a run / lease a job /
//! record an outcome / replay a bundle" into store and engine calls. The CLI,
//! the REST server, remote workers and the MCP server all go through it.
mod error;
mod types;

pub use error::ServiceError;
pub use types::*;

use anthill_core::bundle::Bundle;
use anthill_engine::backend::Progress;
use anthill_engine::config::{Budget, TargetKind};
use anthill_engine::engine::{Engine, RunRequest};
use anthill_engine::report::{ReplayOutcome, RunOutcome, RunStatus, Step};
use anthill_store::{
    now_ms, random_seed, EventRecord, FailureRecord, JobStatus, NewFailure, NewRun, RunRecord,
    RunState, Store, WorkerRecord,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

/// Largest bundle text accepted from a caller.
pub const MAX_BUNDLE_BYTES: usize = 1024 * 1024;
const MAX_TARGET_LEN: usize = 64;
const LEASE_SECS: u64 = 120;
const MAX_LIST: u32 = 500;
const MAX_EVENTS: u32 = 1000;
const JOB_KIND_RUN: &str = "run";

pub type Result<T> = std::result::Result<T, ServiceError>;

#[derive(Clone)]
pub struct Service {
    store: Arc<dyn Store>,
}

fn backend_name(kind: &TargetKind) -> &'static str {
    match kind {
        TargetKind::Sim { .. } => "sim",
        TargetKind::Process { .. } => "hermit",
        TargetKind::Wasm { .. } => "wasm",
    }
}

fn summary(f: &FailureRecord) -> FailureSummary {
    FailureSummary {
        id: f.id.clone(),
        run_id: f.run_id.clone(),
        target: f.target.clone(),
        message: f.message.clone(),
        original_len: f.original_len,
        shrunk_len: f.shrunk_len,
        bundle_hash: f.bundle_hash.clone(),
        created_at: f.created_at,
    }
}

impl Service {
    pub fn new(store: Arc<dyn Store>) -> Service {
        Service { store }
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    pub fn list_targets(&self, engine: &Engine) -> Vec<TargetInfo> {
        engine
            .config()
            .targets
            .values()
            .map(|t| TargetInfo {
                name: t.name.clone(),
                kind: t.kind.name().to_string(),
                description: t.description.clone(),
                budget: t.budget,
            })
            .collect()
    }

    pub fn start_run(&self, engine: &Engine, req: &StartRun) -> Result<RunRecord> {
        if req.target.is_empty() || req.target.len() > MAX_TARGET_LEN {
            return Err(ServiceError::InvalidRequest(
                "target must be 1-64 characters".to_string(),
            ));
        }
        let spec = engine.config().targets.get(&req.target).ok_or_else(|| {
            ServiceError::InvalidRequest(format!("unknown target `{}`", req.target))
        })?;
        let payload = JobPayload {
            target: req.target.clone(),
            seed: req.seed.unwrap_or_else(random_seed),
            budget: req.budget.map(Budget::clamp),
        };
        let json = serde_json::to_string(&payload)
            .map_err(|e| ServiceError::InvalidRequest(e.to_string()))?;
        let now = now_ms();
        let run = self.store.create_run(
            &NewRun {
                target: req.target.clone(),
                backend: backend_name(&spec.kind).to_string(),
                seed: payload.seed,
                config_json: json.clone(),
            },
            now,
        )?;
        self.store.enqueue_job(&run.id, JOB_KIND_RUN, &json, now)?;
        Ok(run)
    }

    pub fn get_run(&self, id: &str) -> Result<Option<RunDetail>> {
        let Some(run) = self.store.get_run(id)? else {
            return Ok(None);
        };
        let failures = self.store.list_failures(id)?.iter().map(summary).collect();
        Ok(Some(RunDetail { run, failures }))
    }

    pub fn list_runs(&self, limit: u32) -> Result<Vec<RunRecord>> {
        Ok(self.store.list_runs(limit.clamp(1, MAX_LIST))?)
    }

    pub fn recent_failures(&self, limit: u32) -> Result<Vec<FailureSummary>> {
        Ok(self
            .store
            .list_recent_failures(limit.clamp(1, MAX_LIST))?
            .iter()
            .map(summary)
            .collect())
    }

    pub fn get_failure(&self, id: &str) -> Result<Option<FailureDetail>> {
        let Some(record) = self.store.get_failure(id)? else {
            return Ok(None);
        };
        let steps: Vec<Step> = serde_json::from_str(&record.steps_json)
            .map_err(|e| ServiceError::Internal(format!("corrupt steps: {e}")))?;
        let bundle = self.get_bundle(&record.bundle_hash)?.unwrap_or_default();
        Ok(Some(FailureDetail {
            record: summary(&record),
            steps,
            bundle,
        }))
    }

    pub fn get_bundle(&self, hash: &str) -> Result<Option<String>> {
        Ok(self
            .store
            .get_blob(hash)?
            .map(|b| String::from_utf8_lossy(&b).into_owned()))
    }

    /// Rebuilds a [`RunOutcome`] from stored data so stored runs can be rendered like fresh ones.
    pub fn outcome(&self, run_id: &str) -> Result<Option<RunOutcome>> {
        let Some(detail) = self.get_run(run_id)? else {
            return Ok(None);
        };
        let run = detail.run;
        let status = match run.state {
            RunState::Passed => RunStatus::Passed,
            RunState::Failed => RunStatus::Failed,
            _ => RunStatus::Error,
        };
        let failure = match detail.failures.first() {
            Some(f) => self
                .get_failure(&f.id)?
                .map(|d| anthill_engine::report::FailureReport {
                    target: d.record.target,
                    backend: run.backend.clone(),
                    message: d.record.message,
                    original_len: d.record.original_len,
                    shrunk_len: d.record.shrunk_len,
                    steps: d.steps,
                    bundle: d.bundle,
                }),
            None => None,
        };
        let elapsed_ms = match (run.started_at, run.finished_at) {
            (Some(s), Some(f)) if f >= s => (f - s) as u64,
            _ => 0,
        };
        let error = run.error.clone().or_else(|| {
            (status == RunStatus::Error)
                .then(|| format!("run ended in state `{}`", run.state.as_str()))
        });
        Ok(Some(RunOutcome {
            status,
            steps_executed: run.steps,
            trials: run.trials,
            elapsed_ms,
            failure,
            error,
        }))
    }

    pub fn events(&self, run_id: &str, after_id: i64, limit: u32) -> Result<Vec<EventRecord>> {
        Ok(self
            .store
            .list_events(run_id, after_id, limit.clamp(1, MAX_EVENTS))?)
    }

    pub fn cancel_run(&self, id: &str) -> Result<bool> {
        Ok(self.store.cancel_run(id, now_ms())?)
    }

    pub fn register_worker(&self, name: &str, caps_json: &str) -> Result<WorkerRecord> {
        if name.is_empty() || name.len() > MAX_TARGET_LEN {
            return Err(ServiceError::InvalidRequest(
                "worker name must be 1-64 characters".to_string(),
            ));
        }
        Ok(self.store.register_worker(name, caps_json, now_ms())?)
    }

    pub fn list_workers(&self) -> Result<Vec<WorkerRecord>> {
        Ok(self.store.list_workers()?)
    }

    pub fn touch_worker(&self, id: &str) -> Result<()> {
        Ok(self.store.touch_worker(id, now_ms())?)
    }

    pub fn replay_bundle(&self, engine: &Engine, text: &str) -> Result<ReplayOutcome> {
        if text.len() > MAX_BUNDLE_BYTES {
            return Err(ServiceError::InvalidRequest(format!(
                "bundle exceeds {MAX_BUNDLE_BYTES} bytes"
            )));
        }
        let bundle =
            Bundle::decode(text).map_err(|e| ServiceError::InvalidRequest(e.to_string()))?;
        Ok(engine.replay(&bundle)?)
    }

    // ---- job execution ----

    /// Leases the next runnable job and marks its run as started.
    pub fn lease(&self, _engine: &Engine, worker_id: &str) -> Result<Option<LeasedJob>> {
        loop {
            let Some(job) = self.store.lease_job(worker_id, LEASE_SECS, now_ms())? else {
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
                    self.store
                        .fail_job(&job.id, worker_id, "run no longer exists", now_ms())?;
                    continue;
                }
                Some(_) => {}
            }
            let now = now_ms();
            self.store.mark_running(&job.run_id, now)?;
            self.store.add_event(
                &job.run_id,
                "started",
                &format!("{{\"worker\":{}}}", serde_json::json!(worker_id)),
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
            .heartbeat_job(job_id, worker_id, LEASE_SECS, now_ms())?)
    }

    pub fn report_progress(&self, run_id: &str, p: &Progress) -> Result<()> {
        self.store.update_progress(run_id, p.steps, p.trials)?;
        let payload =
            serde_json::json!({ "steps": p.steps, "trials": p.trials, "message": p.message });
        self.store
            .add_event(run_id, "progress", &payload.to_string(), now_ms())?;
        Ok(())
    }

    /// Progress from a (possibly remote) worker; only the current lease holder may report.
    pub fn report_job_progress(&self, job_id: &str, worker_id: &str, p: &Progress) -> Result<()> {
        let job = self
            .store
            .get_job(job_id)?
            .ok_or_else(|| ServiceError::NotFound(format!("job {job_id}")))?;
        if job.status != JobStatus::Running || job.lease_owner.as_deref() != Some(worker_id) {
            return Err(ServiceError::InvalidRequest(
                "job is not leased by this worker".to_string(),
            ));
        }
        self.report_progress(&job.run_id, p)?;
        self.heartbeat(job_id, worker_id)?;
        Ok(())
    }

    /// Records the outcome of a leased job (used by local and remote workers alike).
    pub fn complete(&self, job_id: &str, worker_id: &str, outcome: &RunOutcome) -> Result<()> {
        let job = self
            .store
            .get_job(job_id)?
            .ok_or_else(|| ServiceError::NotFound(format!("job {job_id}")))?;
        if job.status != JobStatus::Running || job.lease_owner.as_deref() != Some(worker_id) {
            return Err(ServiceError::InvalidRequest(
                "job is not leased by this worker".to_string(),
            ));
        }
        let now = now_ms();
        let state = match outcome.status {
            RunStatus::Passed => RunState::Passed,
            RunStatus::Failed => RunState::Failed,
            RunStatus::Error => RunState::Error,
        };
        if let Some(f) = &outcome.failure {
            let hash = self.store.put_blob(f.bundle.as_bytes())?;
            let steps_json = serde_json::to_string(&f.steps)
                .map_err(|e| ServiceError::Internal(e.to_string()))?;
            self.store.add_failure(
                &NewFailure {
                    run_id: job.run_id.clone(),
                    target: f.target.clone(),
                    message: f.message.clone(),
                    original_len: f.original_len,
                    shrunk_len: f.shrunk_len,
                    bundle_hash: hash,
                    steps_json,
                },
                now,
            )?;
        }
        // A run cancelled while executing keeps its Cancelled state.
        if let Err(e) = self.store.finish_run(
            &job.run_id,
            state,
            outcome.steps_executed,
            outcome.trials,
            outcome.error.as_deref(),
            now,
        ) {
            if self.store.get_run(&job.run_id)?.map(|r| r.state) != Some(RunState::Cancelled) {
                return Err(e.into());
            }
        }
        self.store.add_event(
            &job.run_id,
            "finished",
            &serde_json::json!({ "state": state.as_str() }).to_string(),
            now,
        )?;
        self.store.complete_job(job_id, worker_id)?;
        Ok(())
    }

    /// Records a transient worker failure; the job is retried until attempts run out.
    pub fn fail(&self, job_id: &str, worker_id: &str, error: &str) -> Result<()> {
        let now = now_ms();
        self.store.fail_job(job_id, worker_id, error, now)?;
        if let Some(job) = self.store.get_job(job_id)? {
            if job.status == JobStatus::Failed {
                let _ = self
                    .store
                    .finish_run(&job.run_id, RunState::Error, 0, 0, Some(error), now);
                self.store.add_event(
                    &job.run_id,
                    "finished",
                    &serde_json::json!({ "state": "error" }).to_string(),
                    now,
                )?;
            }
        }
        Ok(())
    }

    /// Leases and executes one job locally. Returns false when the queue is empty.
    pub fn execute_next(&self, engine: &Engine, worker_id: &str) -> Result<bool> {
        let Some(job) = self.lease(engine, worker_id)? else {
            return Ok(false);
        };
        let req = RunRequest {
            target: job.target.clone(),
            seed: job.seed,
            budget: job.budget,
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            engine.run(&req, &mut |p| {
                // Progress and heartbeat are best effort; a storage hiccup must not kill the run.
                let _ = self.report_progress(&job.run_id, &p);
                let _ = self.heartbeat(&job.job_id, worker_id);
            })
        }));
        match result {
            Ok(Ok(outcome)) => self.complete(&job.job_id, worker_id, &outcome)?,
            Ok(Err(e)) => {
                self.complete(&job.job_id, worker_id, &RunOutcome::error(e.to_string()))?
            }
            Err(_) => self.fail(
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
        loop {
            let detail = self
                .get_run(&run.id)?
                .ok_or_else(|| ServiceError::NotFound(run.id.clone()))?;
            if detail.run.state.is_terminal() {
                return Ok(detail);
            }
            if !self.execute_next(engine, "local")? {
                return Err(ServiceError::Internal(
                    "queue drained but the run is not finished".to_string(),
                ));
            }
        }
    }
}
