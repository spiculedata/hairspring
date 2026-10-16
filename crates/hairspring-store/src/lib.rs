//! Hairspring persistence layer.
//!
//! One [`Store`] trait, implemented for SQLite (default, zero config) and
//! Postgres (added in a later phase). All time values are milliseconds since the
//! Unix epoch and are passed in explicitly so behaviour is testable.
#[cfg(feature = "postgres")]
mod pg;
#[cfg(feature = "postgres")]
mod pg_schema;
#[cfg(feature = "postgres")]
mod pg_trials;
mod sqlite;
mod sqlite_schema;
mod sqlite_trials;
#[macro_use]
mod types;
mod trials;

#[cfg(feature = "postgres")]
pub use pg::PgStore;
pub use sqlite::SqliteStore;
pub use trials::*;
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

/// Seeds are limited to 53 bits so JSON consumers that parse numbers as doubles (browsers, LLM tool
/// clients) see exactly the value we stored.
const JS_SAFE_MASK: u64 = (1 << 53) - 1;

/// Unpredictable-enough seed for runs that do not specify one (not for secrets).
pub fn random_seed() -> u64 {
    let id = new_id("seed");
    let raw = u64::from_str_radix(id.trim_start_matches("seed_"), 16).unwrap_or(0x5eed);
    raw & JS_SAFE_MASK
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
    /// Moves a queued or running run to a final state. Returns false (changing nothing) if it was already final.
    fn finish_run(
        &self,
        id: &str,
        state: RunState,
        steps: u64,
        trials: u64,
        error: Option<&str>,
        now: i64,
    ) -> Result<bool>;
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
    /// Fails running jobs whose lease expired after their last permitted attempt, and reports them
    /// (exactly once) so their runs can be finalised.
    fn fail_expired_jobs(&self, now: i64) -> Result<Vec<ExpiredJob>>;
    /// Atomically records a worker's result: failure row, run state and `finished` event, and closes the job.
    fn finish_job(
        &self,
        id: &str,
        worker_id: &str,
        result: &JobResult,
        now: i64,
    ) -> Result<FinishOutcome>;
    fn queued_job_count(&self) -> Result<u64>;
    /// Records a failure; requeues the job until [`MAX_JOB_ATTEMPTS`] is reached.
    fn fail_job(&self, id: &str, worker_id: &str, error: &str, now: i64) -> Result<bool>;

    // events
    fn add_event(&self, run_id: &str, kind: &str, payload_json: &str, now: i64) -> Result<i64>;
    fn list_events(&self, run_id: &str, after_id: i64, limit: u32) -> Result<Vec<EventRecord>>;

    // trials, the run catalog and per-property statistics (schema version 2)
    /// Stores one batch of catalog changes and trial rows for the job's run, all or nothing. Idempotent on `seq`: a
    /// row already stored with identical content is counted as a duplicate; one that differs rejects the batch.
    /// `worker_trusted` is recorded on every row as a snapshot.
    fn ingest_trials(
        &self,
        job_id: &str,
        worker_id: &str,
        worker_trusted: bool,
        batch: &TrialBatch,
        now: i64,
    ) -> Result<IngestOutcome>;
    fn count_trials(&self, run_id: &str) -> Result<u64>;
    fn get_trial(&self, run_id: &str, seq: u64) -> Result<Option<TrialRecord>>;
    /// Trial rows in sequence order, after `after_seq` when given.
    fn list_trials(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: u32,
    ) -> Result<Vec<TrialRecord>>;
    /// Only what the run fold needs, in sequence order, after `after_seq` when given.
    fn list_trial_digests(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: u32,
    ) -> Result<Vec<TrialDigestRow>>;
    fn get_catalog(&self, run_id: &str) -> Result<Vec<CatalogRowRecord>>;
    fn list_run_properties(&self, run_id: &str) -> Result<Vec<RunPropertyRow>>;
    /// The workers that reported trials for the run, with whether all of their rows were from a trusted worker.
    fn run_contributors(&self, run_id: &str) -> Result<Vec<Contributor>>;

    // workers
    fn set_worker_trusted(&self, id: &str, trusted: bool) -> Result<()>;
    /// `None` for an unknown worker.
    fn worker_trusted(&self, id: &str) -> Result<Option<bool>>;
    /// Stores the hash of the secret issued to a worker at registration. The secret itself is never stored.
    fn set_worker_secret_hash(&self, id: &str, hash: &str) -> Result<()>;
    /// `None` for an unknown worker or one registered without a secret.
    fn worker_secret_hash(&self, id: &str) -> Result<Option<String>>;
    fn register_worker(&self, name: &str, caps_json: &str, now: i64) -> Result<WorkerRecord>;
    fn touch_worker(&self, id: &str, now: i64) -> Result<()>;
    fn list_workers(&self) -> Result<Vec<WorkerRecord>>;
    fn worker_count(&self) -> Result<u64>;
    /// Deletes workers not seen since `before`; returns how many were removed.
    fn prune_workers(&self, before: i64) -> Result<u64>;
}
