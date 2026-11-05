//! A read-only view of the workers: whether each is busy, idle or has gone quiet, what it is trusted for, what it said
//! it can do, which jobs it holds, and how many jobs wait behind them.
//!
//! Everything here comes from rows the store already keeps: the worker table and the live leases on the job table. No
//! secret, secret hash, job payload, job error or guest text is read, so none can be returned.
use crate::{Result, Service};
use hairspring_store::{now_ms, WorkerRecord};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// Held jobs named for each worker. The count of all of them is reported next to the names.
pub const MAX_CURRENT_JOBS_SHOWN: usize = 3;
/// The most live leases read in one call. A fleet that holds more than this at once is shown a lower count per worker;
/// the queue's running figure is always exact.
const MAX_LEASES_READ: u32 = 5_000;

/// What a worker is doing, judged from its live leases and when the server last heard from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    /// Holds at least one lease that has not ended.
    Busy,
    /// Holds nothing and was heard from within one lease length.
    Idle,
    /// Holds nothing and was last heard from more than one lease length ago.
    NotSeenRecently,
}

/// One job a worker holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerJob {
    pub run_id: String,
    pub kind: String,
    pub lease_until: i64,
    pub attempts: u32,
}

/// A worker as the page shows it. The record's own fields come first, flattened, so the existing keys are unchanged.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkerView {
    #[serde(flatten)]
    pub worker: WorkerRecord,
    pub state: WorkerState,
    /// The capabilities the worker sent, when they are valid JSON; `None` otherwise (the raw text is in `caps_json`).
    pub capabilities: Option<Value>,
    /// Up to [`MAX_CURRENT_JOBS_SHOWN`] of the jobs held, oldest first.
    pub current_jobs: Vec<WorkerJob>,
    pub leased_jobs: u64,
}

/// Jobs waiting for a worker and jobs a worker holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct QueueSummary {
    pub queued: u64,
    pub running: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkersOverview {
    pub workers: Vec<WorkerView>,
    pub queue: QueueSummary,
}

impl Service {
    /// The workers and the queue as of now.
    pub fn workers_overview(&self) -> Result<WorkersOverview> {
        self.workers_overview_at(now_ms())
    }

    /// The workers and the queue as of `now` (milliseconds). Public so that tests can fix the time.
    pub fn workers_overview_at(&self, now: i64) -> Result<WorkersOverview> {
        let workers = self.store.list_workers()?;
        let leases = self.store.leased_jobs(now, MAX_LEASES_READ)?;
        let queued = self.store.queued_job_count()?;
        let running = self.store.leased_job_count(now)?;
        let window = (self.cfg.lease_secs as i64).saturating_mul(1000);
        let mut held: HashMap<String, Vec<WorkerJob>> = HashMap::new();
        let mut held_counts: HashMap<String, u64> = HashMap::new();
        for lease in leases {
            *held_counts.entry(lease.lease_owner.clone()).or_default() += 1;
            let jobs = held.entry(lease.lease_owner).or_default();
            if jobs.len() < MAX_CURRENT_JOBS_SHOWN {
                jobs.push(WorkerJob {
                    run_id: lease.run_id,
                    kind: lease.kind,
                    lease_until: lease.lease_until,
                    attempts: lease.attempts,
                });
            }
        }
        let views = workers
            .into_iter()
            .map(|worker| {
                let leased_jobs = held_counts.get(&worker.id).copied().unwrap_or(0);
                let state = if leased_jobs > 0 {
                    WorkerState::Busy
                } else if now.saturating_sub(worker.last_seen) <= window {
                    WorkerState::Idle
                } else {
                    WorkerState::NotSeenRecently
                };
                let capabilities = serde_json::from_str(&worker.caps_json).ok();
                let current_jobs = held.remove(&worker.id).unwrap_or_default();
                WorkerView {
                    worker,
                    state,
                    capabilities,
                    current_jobs,
                    leased_jobs,
                }
            })
            .collect();
        Ok(WorkersOverview {
            workers: views,
            queue: QueueSummary { queued, running },
        })
    }
}
