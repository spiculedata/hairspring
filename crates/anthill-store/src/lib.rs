//! Anthill persistence layer.
//!
//! One [`Store`] trait, implemented for SQLite (default, zero config) and
//! Postgres (feature `postgres`). All time values are milliseconds since the
//! Unix epoch and are passed in explicitly so behaviour is testable.
mod sqlite;
mod types;

pub use sqlite::SqliteStore;
pub use types::*;

use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Jobs are retried until this many attempts have been made.
pub const MAX_JOB_ATTEMPTS: u32 = 3;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Collision-resistant (not secret) identifier with a readable prefix.
pub fn new_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = Sha256::new();
    h.update(nanos.to_le_bytes());
    h.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    let digest = h.finalize();
    format!("{prefix}_{}", hex(&digest[..8]))
}

/// Unpredictable-enough seed for runs that do not specify one (not for secrets).
pub fn random_seed() -> u64 {
    let id = new_id("seed");
    u64::from_str_radix(id.trim_start_matches("seed_"), 16).unwrap_or(0x5eed)
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub trait Store: Send + Sync {
    // runs
    fn create_run(&self, new: &NewRun, now: i64) -> Result<RunRecord>;
    fn get_run(&self, id: &str) -> Result<Option<RunRecord>>;
    fn list_runs(&self, limit: u32) -> Result<Vec<RunRecord>>;
    fn mark_running(&self, id: &str, now: i64) -> Result<()>;
    fn update_progress(&self, id: &str, steps: u64, trials: u64) -> Result<()>;
    fn finish_run(
        &self,
        id: &str,
        state: RunState,
        steps: u64,
        trials: u64,
        error: Option<&str>,
        now: i64,
    ) -> Result<()>;
    /// Cancels a queued or running run. Returns false if it was already terminal.
    fn cancel_run(&self, id: &str, now: i64) -> Result<bool>;

    // failures and blobs
    fn add_failure(&self, new: &NewFailure, now: i64) -> Result<FailureRecord>;
    fn get_failure(&self, id: &str) -> Result<Option<FailureRecord>>;
    fn list_failures(&self, run_id: &str) -> Result<Vec<FailureRecord>>;
    fn list_recent_failures(&self, limit: u32) -> Result<Vec<FailureRecord>>;
    fn put_blob(&self, data: &[u8]) -> Result<String>;
    fn get_blob(&self, hash: &str) -> Result<Option<Vec<u8>>>;

    // job queue
    fn enqueue_job(&self, run_id: &str, kind: &str, payload_json: &str, now: i64) -> Result<Job>;
    fn get_job(&self, id: &str) -> Result<Option<Job>>;
    /// Leases the oldest runnable job (queued, or running with an expired lease).
    fn lease_job(&self, worker_id: &str, lease_secs: u64, now: i64) -> Result<Option<Job>>;
    fn heartbeat_job(&self, id: &str, worker_id: &str, lease_secs: u64, now: i64) -> Result<bool>;
    fn complete_job(&self, id: &str, worker_id: &str) -> Result<bool>;
    /// Records a failure; requeues the job until [`MAX_JOB_ATTEMPTS`] is reached.
    fn fail_job(&self, id: &str, worker_id: &str, error: &str, now: i64) -> Result<bool>;

    // events
    fn add_event(&self, run_id: &str, kind: &str, payload_json: &str, now: i64) -> Result<i64>;
    fn list_events(&self, run_id: &str, after_id: i64, limit: u32) -> Result<Vec<EventRecord>>;

    // workers
    fn register_worker(&self, name: &str, caps_json: &str, now: i64) -> Result<WorkerRecord>;
    fn touch_worker(&self, id: &str, now: i64) -> Result<()>;
    fn list_workers(&self) -> Result<Vec<WorkerRecord>>;
}
