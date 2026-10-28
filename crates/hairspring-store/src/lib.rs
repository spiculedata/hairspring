//! Hairspring persistence layer.
//!
//! One [`Store`] trait, implemented for SQLite (default, zero config) and
//! Postgres (added in a later phase). All time values are milliseconds since the
//! Unix epoch and are passed in explicitly so behaviour is testable.
#[macro_use]
mod types;
mod checks;
mod digest_allowance;
mod downgrade;
#[macro_use]
mod lineage;
mod lineage_fields;
#[cfg(feature = "postgres")]
mod pg;
#[cfg(feature = "postgres")]
mod pg_checks;
#[cfg(feature = "postgres")]
mod pg_downgrade;
#[cfg(feature = "postgres")]
mod pg_explorer;
#[cfg(feature = "postgres")]
mod pg_lineage;
#[cfg(feature = "postgres")]
mod pg_run_end;
#[cfg(feature = "postgres")]
mod pg_schema;
#[cfg(feature = "postgres")]
mod pg_tree_query;
#[cfg(feature = "postgres")]
mod pg_trials;
mod reporters;
mod sqlite;
mod sqlite_checks;
mod sqlite_downgrade;
mod sqlite_explorer;
mod sqlite_lineage;
mod sqlite_run_end;
mod sqlite_schema;
mod sqlite_tree_query;
mod sqlite_trials;
mod sqlite_tx;
mod trials;

pub use checks::*;
pub use digest_allowance::*;
pub use downgrade::*;
pub use lineage::*;
#[cfg(feature = "postgres")]
pub use pg::PgStore;
#[cfg(feature = "postgres")]
pub use pg_downgrade::downgrade_postgres;
pub use sqlite::{SqliteStore, BUSY_TIMEOUT};
pub use sqlite_downgrade::downgrade_sqlite;
pub use trials::*;
pub use types::*;

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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
    /// Cancels a queued or running run. Returns false if it was already terminal. This writes no event: callers that
    /// report the cancellation to readers of the run's event log must use [`Store::cancel_run_with_event`], so the
    /// state and the `cancelled` event become visible together.
    fn cancel_run(&self, id: &str, now: i64) -> Result<bool>;
    /// [`Store::cancel_run`] and the run's `event`, committed together: a reader that sees the run cancelled also sees
    /// the event. The event is written only when the run was cancelled. Returns false for an unknown or final run.
    fn cancel_run_with_event(&self, id: &str, event: &NewEvent<'_>, now: i64) -> Result<bool>;
    /// Moves a queued or running run to the error state with `reason`, and appends `event`, in one transaction. The
    /// event is written only when the run was moved. Returns false (changing nothing) if it was already final, and
    /// `NotFound` for an unknown run.
    fn finish_run_as_error(
        &self,
        id: &str,
        reason: &str,
        event: &NewEvent<'_>,
        now: i64,
    ) -> Result<bool>;

    // failures and blobs
    fn add_failure(&self, new: &NewFailure, now: i64) -> Result<FailureRecord>;
    fn get_failure(&self, id: &str) -> Result<Option<FailureRecord>>;
    fn list_failures(&self, run_id: &str) -> Result<Vec<FailureRecord>>;
    fn list_recent_failures(&self, limit: u32) -> Result<Vec<FailureRecord>>;
    fn put_blob(&self, data: &[u8]) -> Result<String>;
    fn get_blob(&self, hash: &str) -> Result<Option<Vec<u8>>>;

    // the stored results of confirming a failure (schema version 5)
    /// Stores one confirmation's checks all together or not at all. The same confirmation again (same id, same
    /// content) is a `Duplicate` and changes nothing; one that differs under the same id is refused. An unknown failure
    /// is `NotFound`; more checks than [`MAX_CHECKS_PER_CONFIRMATION`], or than [`MAX_CHECKS_PER_FAILURE`] held for the
    /// failure, or a column over [`MAX_CHECK_TEXT_BYTES`], is `LimitExceeded`. The store keeps what it is given: the
    /// service checks what a runner reports before calling this.
    fn add_confirmation(&self, new: &NewConfirmation, now: i64) -> Result<ConfirmationOutcome>;
    /// The checks held for a failure, oldest confirmation first and in each confirmation's order.
    fn list_failure_checks(&self, failure_id: &str) -> Result<Vec<CheckRecord>>;

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
    ///
    /// The digest allowance (`batch.digest_bytes_cap`) is checked under the run's lock. A trial whose digest would pass
    /// it is stored as its `on_digest_cap` stand-in. The store **trusts that stand-in** (it checks only that it is the
    /// same sequence number and seed and passes the field limits, not what the digest means), so only code that has
    /// validated the original may build it. With none offered the answer is `NeedsStandIn` and nothing is written. A
    /// re-sent trial is compared with the row as it was stored: equal to the trial, or to its offered stand-in.
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
    /// Only the verdict and flags of each trial, in sequence order, after `after_seq` when given. It never reads the
    /// digest, so its cost does not depend on how large the stored digests are.
    fn list_trial_strip_rows(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: u32,
    ) -> Result<Vec<TrialStripRow>>;
    fn get_catalog(&self, run_id: &str) -> Result<Vec<CatalogRowRecord>>;
    fn list_run_properties(&self, run_id: &str) -> Result<Vec<RunPropertyRow>>;
    /// The workers that reported trials for the run, with whether all of their rows were from a trusted worker.
    fn run_contributors(&self, run_id: &str) -> Result<Vec<Contributor>>;

    // the explorer's storage needs (schema version 4)
    /// The worker that first reported each of the given seeds in the run (the lowest sequence number wins). A seed
    /// the run has no trial for is left out. Reads only the seed and worker columns, never a whole trial row. More
    /// than [`MAX_REPORTER_SEEDS`] seeds is `LimitExceeded`.
    fn reporters_of(&self, run_id: &str, seeds: &[u64]) -> Result<BTreeMap<u64, String>>;
    /// How much of the run's digest allowance its stored trials use. Zero for an unknown run or one with no digests.
    fn run_digest_use(&self, run_id: &str) -> Result<DigestUse>;
    /// The coverage series stored when the run's job finished, or `None` (an unfinished run, or one finished before
    /// the series was stored).
    fn run_coverage_json(&self, run_id: &str) -> Result<Option<String>>;

    // run identity and trial trees (schema version 3)
    /// Records the execution inputs of a run. The same identity again is a duplicate and a different one is refused,
    /// with one exception: an incomplete identity is replaced by a complete one (`Completed`) while no tree is rooted
    /// at, or branches from, the run. A complete identity never changes. Completion is accepted only from the
    /// reporter that wrote the incomplete identity (another reporter's offer is a conflict), and the stored trust
    /// flag is never raised: the completed row is trusted only if both the stored and the offered report were.
    fn put_run_identity(&self, identity: &RunIdentity) -> Result<IdentityOutcome>;
    fn get_run_identity(&self, run_id: &str) -> Result<Option<RunIdentity>>;
    /// Creates the tree rooted at a run. `identity_hash` must be the root run's identity hash, or `None` when the run
    /// has no complete identity (a tree without point identity: reads work, branches are refused).
    fn create_tree(&self, new: &NewTree, now: i64) -> Result<TreeOutcome>;
    fn get_tree(&self, id: &str) -> Result<Option<TreeRecord>>;
    /// The tree a run roots or branches from.
    fn tree_of_run(&self, run_id: &str) -> Result<Option<String>>;
    /// Attaches a stored trial to the tree. Number, depth, path and verdict are derived here, never taken from the
    /// caller; the depth, fan-out and node caps are enforced. An `edge_ref`, when given, is a SHA-256 digest in
    /// lower-case hexadecimal (64 characters); anything else is `Rejected(Invalid)`.
    fn add_tree_node(&self, tree_id: &str, new: &NewNode, now: i64) -> Result<NodeOutcome>;
    fn get_tree_node(&self, tree_id: &str, node: u64) -> Result<Option<NodeRecord>>;
    fn tree_node_of_trial(&self, run_id: &str, seq: u64) -> Result<Option<NodeRecord>>;
    /// The one mutable column of a node. Fails with `NotFound` for an unknown node.
    fn set_node_verified(&self, tree_id: &str, node: u64, verified: Verified) -> Result<()>;
    /// Chooses how subtree queries are answered (see [`TreeQueryRule`]). The answers do not depend on it; only the
    /// cost does. The default is each store's own rule. Process-wide tuning and tests only: **never wire this to a
    /// route, tool or any request-driven path** (a caller could force the expensive form).
    #[doc(hidden)]
    fn set_tree_query_rule(&self, rule: TreeQueryRule);
    #[doc(hidden)]
    fn tree_query_rule(&self) -> TreeQueryRule;
    /// The descendants of a node (not the node), in node order. Every tree list uses one cursor rule: the cursor is a
    /// decimal node number and lists are in node order. In the recursive form (and for children) the next cursor is
    /// the last node returned when more exist, else `None`. In the path form a request scans at most [`NODE_WINDOW`]
    /// numbers after the cursor, and the next cursor is the last node returned if the page filled, else the end of
    /// the window; `None` once no later node can exist. In the recursive form a node with more than
    /// [`MAX_CTE_DESCENDANTS`] descendants is answered with [`StoreError::LimitExceeded`]: page by children.
    fn subtree_descendants(
        &self,
        tree_id: &str,
        node: u64,
        after: Option<u64>,
        limit: u32,
    ) -> Result<NodePage>;
    /// Verdict counts of the node and all of its descendants.
    fn subtree_tally(&self, tree_id: &str, node: u64) -> Result<Vec<VerdictCount>>;
    /// The shallowest, then earliest, violating node at or below the node.
    fn subtree_first_failing(&self, tree_id: &str, node: u64) -> Result<Option<u64>>;
    /// The children of a node, in node order, by keyset paging on the parent index (no window: siblings can be far
    /// apart in node numbers).
    fn tree_children(
        &self,
        tree_id: &str,
        node: u64,
        after: Option<u64>,
        limit: u32,
    ) -> Result<NodePage>;
    /// The nodes above a node, root first (not the node itself).
    fn tree_ancestors(&self, tree_id: &str, node: u64) -> Result<Vec<NodeRecord>>;
    /// Registers a branch run of a tree. A command already stored for the tree answers `Existing`. `point_hash` and
    /// `command_hash` are SHA-256 digests in lower-case hexadecimal (64 characters); `command_json` is one
    /// well-formed JSON document of at most [`MAX_COMMAND_JSON_BYTES`] bytes and [`MAX_COMMAND_JSON_DEPTH`] levels of
    /// nesting. Anything else is `Rejected(Invalid)` and nothing is written.
    fn add_tree_branch(&self, new: &NewBranch, now: i64) -> Result<BranchOutcome>;
    fn get_tree_branch(&self, run_id: &str) -> Result<Option<BranchRecord>>;
    fn find_tree_branch(&self, tree_id: &str, command_hash: &str) -> Result<Option<BranchRecord>>;
    /// Counts one verification re-run of a branch, and whether it matched.
    fn record_branch_verify(&self, run_id: &str, matched: bool) -> Result<()>;

    // workers
    fn set_worker_trusted(&self, id: &str, trusted: bool) -> Result<()>;
    /// `None` for an unknown worker.
    fn worker_trusted(&self, id: &str) -> Result<Option<bool>>;
    /// Registers a worker together with the hash of its secret, in one statement.
    fn register_worker_with_secret(
        &self,
        name: &str,
        caps_json: &str,
        secret_hash: &str,
        now: i64,
    ) -> Result<WorkerRecord>;
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
