//! Service layer: the single place that turns "start a run / lease a job /
//! record an outcome / replay a bundle" into store and engine calls. The CLI,
//! the REST server, remote workers and the MCP server all go through it.
mod claim_view;
pub mod claims;
mod completion;
mod error;
mod ingest;
mod jobs;
mod properties;
mod property_query;
mod query;
pub mod sanitize;
mod scan;
mod trial_query;
mod types;
pub mod view;

pub use claim_view::ClaimView;
pub use completion::{completion_of, RunCompletion};
pub use error::ServiceError;
pub use query::{
    PropertyQuery, SeedQuery, TrialQuery, DEFAULT_PAGE, MAX_PAGE, MAX_PREFIX_BYTES, MAX_SEED_PAGE,
};
pub use types::*;
pub use view::{
    Coverage, PropertyDetail, PropertyPage, PropertyRow, TrialDetail, TrialPage, TrialRow,
};

use hairspring_core::bundle::Bundle;
use hairspring_engine::config::TargetKind;
use hairspring_engine::engine::Engine;
use hairspring_engine::report::{FailureReport, ReplayOutcome, RunOutcome, RunStatus, Step};
use hairspring_store::{
    now_ms, random_seed, EventRecord, FailureRecord, NewRun, RunRecord, RunState, Store,
    WorkerRecord,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Largest bundle text accepted from a caller.
pub const MAX_BUNDLE_BYTES: usize = 1024 * 1024;
/// Longest reproduction path a caller may ask the server to replay.
pub const MAX_REPLAY_STEPS: usize = 20_000;
const MAX_TARGET_LEN: usize = 64;
const MAX_LIST: u32 = 500;
const MAX_EVENTS: u32 = 1000;
const JOB_KIND_RUN: &str = "run";
const STALE_WORKER_AGE_MS: i64 = 24 * 60 * 60 * 1000;

pub type Result<T> = std::result::Result<T, ServiceError>;

/// Tunables with safe defaults.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// How long a worker may hold a job without a heartbeat.
    pub lease_secs: u64,
    /// Runs waiting for a worker; further submissions are refused.
    pub max_queued: u64,
    pub max_workers: u64,
    /// Minimum spacing of stored progress events per run.
    pub progress_event_interval: Duration,
    /// How many bytes of kept trial detail and output one run may hold.
    pub kept_bytes_per_run: u64,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        ServiceConfig {
            lease_secs: 120,
            max_queued: 1000,
            max_workers: 1000,
            progress_event_interval: Duration::from_millis(500),
            kept_bytes_per_run: hairspring_store::MAX_KEPT_BYTES_PER_RUN,
        }
    }
}

#[derive(Clone)]
pub struct Service {
    store: Arc<dyn Store>,
    cfg: Arc<ServiceConfig>,
    progress_seen: Arc<Mutex<HashMap<String, Instant>>>,
}

/// 256 random bits from the operating system, as hex.
fn new_secret() -> Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| ServiceError::Internal(format!("cannot read random bytes: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
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
        Service {
            store,
            cfg: Arc::new(ServiceConfig::default()),
            progress_seen: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_config(self, cfg: ServiceConfig) -> Service {
        Service {
            cfg: Arc::new(cfg),
            ..self
        }
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Lease-owner identity for in-process execution; unique per OS process.
    pub fn local_worker_id() -> String {
        format!("local-{}", std::process::id())
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
        let queued = self.store.queued_job_count()?;
        if queued >= self.cfg.max_queued {
            return Err(ServiceError::InvalidRequest(format!(
                "the queue is full ({queued} runs waiting); try again later"
            )));
        }
        let payload = JobPayload {
            target: req.target.clone(),
            seed: req.seed.unwrap_or_else(random_seed),
            // The budget the run is held to is recorded in full, so it does not depend on the target's default later.
            budget: Some(req.budget.unwrap_or(spec.budget).clamp()),
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
        let properties = self.property_summary(&run)?;
        Ok(Some(RunDetail {
            run,
            failures,
            properties,
        }))
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
        let bundle = self.get_bundle(&record.bundle_hash)?.ok_or_else(|| {
            ServiceError::Internal(format!("bundle blob {} is missing", record.bundle_hash))
        })?;
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
            Some(f) => self.get_failure(&f.id)?.map(|d| FailureReport {
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
        let properties = self.property_stats(run_id)?;
        let property_run = run
            .property_summary_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok());
        Ok(Some(RunOutcome {
            status,
            steps_executed: run.steps,
            trials: run.trials,
            inconclusive: run.inconclusive,
            distinct_schedules: run.distinct_schedules,
            work: run.work,
            elapsed_ms,
            failure,
            error,
            properties,
            property_run,
            trial_log: Vec::new(),
            trials_reported: None,
        }))
    }

    pub fn events(&self, run_id: &str, after_id: i64, limit: u32) -> Result<Vec<EventRecord>> {
        Ok(self
            .store
            .list_events(run_id, after_id, limit.clamp(1, MAX_EVENTS))?)
    }

    pub fn cancel_run(&self, id: &str) -> Result<bool> {
        let now = now_ms();
        let cancelled = self.store.cancel_run(id, now)?;
        if cancelled {
            self.store.add_event(id, "cancelled", "{}", now)?;
        }
        Ok(cancelled)
    }

    /// Checks the name and that there is room for another worker; returns the time to register at.
    fn admit_worker(&self, name: &str) -> Result<i64> {
        if name.is_empty() || name.len() > MAX_TARGET_LEN {
            return Err(ServiceError::InvalidRequest(
                "worker name must be 1-64 characters".to_string(),
            ));
        }
        let now = now_ms();
        if self.store.worker_count()? >= self.cfg.max_workers {
            self.store.prune_workers(now - STALE_WORKER_AGE_MS)?;
            if self.store.worker_count()? >= self.cfg.max_workers {
                return Err(ServiceError::InvalidRequest(
                    "worker limit reached".to_string(),
                ));
            }
        }
        Ok(now)
    }

    /// Registers a worker without a secret. It cannot reach the worker endpoints over HTTP; use
    /// [`Service::register_remote_worker`] for a worker that does.
    pub fn register_worker(&self, name: &str, caps_json: &str) -> Result<WorkerRecord> {
        let now = self.admit_worker(name)?;
        Ok(self.store.register_worker(name, caps_json, now)?)
    }

    /// Registers a remote worker and issues it a secret. The secret is returned once and only its hash is stored;
    /// the worker must present it to send trial records, so that trust attaches to the worker and not to a worker id
    /// that other callers can read.
    pub fn register_remote_worker(
        &self,
        name: &str,
        caps_json: &str,
    ) -> Result<(WorkerRecord, String)> {
        let secret = new_secret()?;
        let now = self.admit_worker(name)?;
        let hash = hairspring_store::sha256_hex(secret.as_bytes());
        let worker = self
            .store
            .register_worker_with_secret(name, caps_json, &hash, now)?;
        Ok((worker, secret))
    }

    /// Registers one of this process's own workers, which is trusted from the start. A remote worker never is: its
    /// trust is set by an administrator with [`Service::set_worker_trust`].
    pub fn register_local_worker(&self, name: &str, caps_json: &str) -> Result<WorkerRecord> {
        let worker = self.register_worker(name, caps_json)?;
        self.store.set_worker_trusted(&worker.id, true)?;
        Ok(WorkerRecord {
            trusted: true,
            ..worker
        })
    }

    /// Sets whether a worker's data may back a claim.
    pub fn set_worker_trust(&self, worker_id: &str, trusted: bool) -> Result<()> {
        match self.store.set_worker_trusted(worker_id, trusted) {
            Err(hairspring_store::StoreError::NotFound(_)) => {
                Err(ServiceError::NotFound(format!("worker {worker_id}")))
            }
            other => Ok(other?),
        }
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
        if bundle.path.len() > MAX_REPLAY_STEPS {
            return Err(ServiceError::InvalidRequest(format!(
                "bundle has {} steps; the replay limit is {MAX_REPLAY_STEPS} steps",
                bundle.path.len()
            )));
        }
        Ok(sanitize::replay_steps(engine.replay(&bundle)?))
    }
}
