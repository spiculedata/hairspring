//! Types for per-trial storage: the run catalog, trial rows, per-property run statistics and worker trust.
//!
//! The store keeps guest-influenced text as plain data and never interprets it; semantic checks (what a digest
//! means, whether a kind is known) belong to the service that calls it. The store still enforces structure and size,
//! because it is the last line before the data is written.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Most trials accepted in one ingest batch.
pub const MAX_TRIALS_PER_BATCH: usize = 256;
/// Most bytes of digests, detail and output accepted in one ingest batch.
pub const MAX_BATCH_BYTES: usize = 1536 * 1024;
/// Largest stored digest, in bytes.
pub const MAX_DIGEST_BLOB_BYTES: usize = 1024 * 1024;
/// Largest kept trial detail, in bytes.
pub const MAX_KEPT_DETAIL_BYTES: usize = 256 * 1024;
/// Largest kept output (both streams together), in bytes.
pub const MAX_KEPT_OUTPUT_BYTES: usize = 128 * 1024;
/// Default allowance of kept detail and output for one run, in bytes.
pub const MAX_KEPT_BYTES_PER_RUN: u64 = 256 * 1024 * 1024;
/// Most catalog rows one run may hold.
pub const MAX_CATALOG_ROWS: usize = 16 * 1024;
/// Longest stored trial reason, in characters.
pub const MAX_TRIAL_REASON_CHARS: usize = 500;
/// Longest stored behaviour hash or schedule fingerprint, in bytes.
pub const MAX_HASH_FIELD_BYTES: usize = 128;
/// Longest catalog id, message and path field, in bytes (the protocol's own limits, with headroom).
pub const MAX_CATALOG_TEXT_BYTES: usize = 1024;
/// Longest catalog kind name, in bytes.
pub const MAX_KIND_NAME_BYTES: usize = 32;

string_enum!(TrialVerdict { Pass => "pass", Violation => "violation", Inconclusive => "inconclusive", Unusable => "unusable", Infra => "infra" });
string_enum!(KeptState { NotKept => "none", Kept => "kept", DroppedCap => "dropped_cap" });

/// One row of the run catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogRowRecord {
    pub idx: u32,
    pub prop_id: String,
    pub kind: String,
    pub must_hit: bool,
    pub msg: String,
    pub loc_file: Option<String>,
    pub loc_line: Option<u64>,
    pub loc_func: Option<String>,
    /// False while only inconclusive trials have mentioned the id.
    pub decided: bool,
}

/// A trial as a worker reports it. `seq` is the trial's position in the run's judging order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTrial {
    pub seq: u64,
    /// The scheduling seed; stored as its two's-complement bit pattern so every `u64` fits.
    pub seed: u64,
    pub verdict: TrialVerdict,
    pub reason: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub degraded: bool,
    pub perturbing: bool,
    pub report_missing: bool,
    pub rated: bool,
    pub claimable: bool,
    pub behaviour_hash: Option<String>,
    pub schedule_fp: Option<String>,
    pub syscalls: u64,
    pub work_ops: u64,
    pub elapsed_ms: u64,
    /// The highest catalog index the digest refers to, so the store can check it against the catalog it holds.
    pub max_idx: Option<u32>,
    /// The encoded trial digest. Opaque to the store.
    pub digest: Vec<u8>,
    /// Kept trial detail (a versioned JSON record), stored in the blob table.
    pub detail: Option<Vec<u8>>,
    /// Kept output tails (JSON), stored in the blob table.
    pub output: Option<Vec<u8>>,
}

/// One ingest request: catalog changes first, then trials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialBatch {
    pub catalog: Vec<CatalogRowRecord>,
    pub trials: Vec<NewTrial>,
    /// How many bytes of kept detail and output the run may hold in all.
    pub kept_bytes_cap: u64,
}

/// A stored trial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialRecord {
    pub run_id: String,
    pub seq: u64,
    pub seed: u64,
    pub verdict: TrialVerdict,
    pub reason: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub degraded: bool,
    pub perturbing: bool,
    pub report_missing: bool,
    pub rated: bool,
    pub claimable: bool,
    pub behaviour_hash: Option<String>,
    pub schedule_fp: Option<String>,
    pub syscalls: u64,
    pub work_ops: u64,
    pub elapsed_ms: u64,
    pub worker_id: String,
    /// Whether the worker was trusted when it reported this trial.
    pub worker_trusted: bool,
    /// The job attempt that produced the row.
    pub attempt: u32,
    pub kept: KeptState,
    pub detail_ref: Option<String>,
    pub output_ref: Option<String>,
    pub digest: Vec<u8>,
}

impl TrialRecord {
    /// Whether this stored row has exactly the content of a re-sent trial. Where it was reported from (worker,
    /// attempt) and what was kept are not part of its content.
    pub fn same_content(&self, t: &NewTrial) -> bool {
        self.seq == t.seq
            && self.seed == t.seed
            && self.verdict == t.verdict
            && self.reason == t.reason
            && self.exit_code == t.exit_code
            && self.signal == t.signal
            && self.timed_out == t.timed_out
            && self.degraded == t.degraded
            && self.perturbing == t.perturbing
            && self.report_missing == t.report_missing
            && self.rated == t.rated
            && self.claimable == t.claimable
            && self.behaviour_hash == t.behaviour_hash
            && self.schedule_fp == t.schedule_fp
            && self.syscalls == t.syscalls
            && self.work_ops == t.work_ops
            && self.elapsed_ms == t.elapsed_ms
            && self.digest == t.digest
    }
}

/// The part of a trial the run fold needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialDigestRow {
    pub seq: u64,
    pub seed: u64,
    pub digest: Vec<u8>,
}

/// The part of a trial the seed strip needs: its verdict and the flags that decide its class. No digest, no text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrialStripRow {
    pub seq: u64,
    pub verdict: TrialVerdict,
    pub rated: bool,
    pub degraded: bool,
    pub report_missing: bool,
}

/// What an ingest request did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    /// Rows written; `duplicates` were already stored with identical content and left alone.
    Applied { inserted: u32, duplicates: u32 },
    /// The caller does not hold the job's lease.
    NotLeased,
    /// The run is already final.
    RunNotOpen,
    /// The batch broke a rule; nothing was written.
    Rejected(String),
}

/// Per-property statistics for one run, as columns. Rates are never stored: `rate_k` and `rate_n` are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunPropertyRow {
    pub idx: u32,
    pub status: String,
    pub trials: u64,
    pub trials_rated: u64,
    pub trials_unreported: u64,
    pub trials_degraded: u64,
    pub trials_perturbing: u64,
    pub trials_conflicted: u64,
    pub trials_inconclusive_with_failures: u64,
    pub trials_claimable: u64,
    pub trials_hit: u64,
    pub trials_true: u64,
    pub trials_false: u64,
    pub evals: u64,
    pub evals_true: u64,
    pub evals_false: u64,
    pub rate_basis: Option<String>,
    pub rate_k: Option<u64>,
    pub rate_n: Option<u64>,
    /// The lowest failing seed among judged trials, as a bit pattern like trial seeds.
    pub first_failure_seed: Option<u64>,
}

/// Run-level figures recorded when a job finishes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunStats {
    pub inconclusive: u64,
    pub distinct_schedules: u64,
    pub work: u64,
    /// The run stopped at its first confirmed failure, so its trial count is not a fixed budget.
    pub stopped_at_first_failure: bool,
    /// The trial rows the server holds do not match what the worker said it ran.
    pub ingest_incomplete: bool,
    pub property_summary_json: Option<String>,
    /// Replaces the run's property rows.
    pub properties: Vec<RunPropertyRow>,
}

/// A worker that reported trials for a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contributor {
    pub worker_id: String,
    /// True only if every trial this worker reported for the run was reported while it was trusted.
    pub trusted: bool,
    pub trials: u64,
}

/// Converts a counter to the signed column type, saturating instead of wrapping.
pub(crate) fn to_db(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn no_nul(s: &str) -> bool {
    !s.contains('\0')
}

fn opt_ok(s: &Option<String>, max: usize) -> bool {
    s.as_deref().is_none_or(|s| s.len() <= max && no_nul(s))
}

impl CatalogRowRecord {
    fn check(&self) -> Result<(), String> {
        let text_ok = |s: &str| s.len() <= MAX_CATALOG_TEXT_BYTES && no_nul(s);
        let ok = !self.prop_id.is_empty()
            && text_ok(&self.prop_id)
            && !self.kind.is_empty()
            && self.kind.len() <= MAX_KIND_NAME_BYTES
            && no_nul(&self.kind)
            && text_ok(&self.msg)
            && opt_ok(&self.loc_file, MAX_CATALOG_TEXT_BYTES)
            && opt_ok(&self.loc_func, MAX_CATALOG_TEXT_BYTES)
            && self.loc_line.is_none_or(|l| l <= i64::MAX as u64);
        if ok {
            Ok(())
        } else {
            Err("a catalog row has an empty or oversized field".to_string())
        }
    }
}

impl NewTrial {
    fn check(&self) -> Result<(), String> {
        let ok = self.seq <= i64::MAX as u64
            && self.syscalls <= i64::MAX as u64
            && self.work_ops <= i64::MAX as u64
            && self.elapsed_ms <= i64::MAX as u64
            && self.reason.chars().count() <= MAX_TRIAL_REASON_CHARS
            && no_nul(&self.reason)
            && opt_ok(&self.behaviour_hash, MAX_HASH_FIELD_BYTES)
            && opt_ok(&self.schedule_fp, MAX_HASH_FIELD_BYTES)
            && self.digest.len() <= MAX_DIGEST_BLOB_BYTES
            && self
                .detail
                .as_ref()
                .is_none_or(|d| d.len() <= MAX_KEPT_DETAIL_BYTES)
            && self
                .output
                .as_ref()
                .is_none_or(|o| o.len() <= MAX_KEPT_OUTPUT_BYTES);
        if ok {
            Ok(())
        } else {
            Err(format!(
                "trial {} has an oversized or invalid field",
                self.seq
            ))
        }
    }

    fn payload_bytes(&self) -> usize {
        self.digest.len()
            + self.detail.as_ref().map_or(0, Vec::len)
            + self.output.as_ref().map_or(0, Vec::len)
    }
}

impl TrialBatch {
    /// Checks the batch's shape and sizes before anything is written. Returns what is wrong, if anything.
    pub fn check(&self) -> Result<(), String> {
        if self.trials.len() > MAX_TRIALS_PER_BATCH {
            return Err(format!(
                "a batch may hold at most {MAX_TRIALS_PER_BATCH} trials"
            ));
        }
        if self.catalog.len() > MAX_CATALOG_ROWS {
            return Err(format!(
                "a batch may hold at most {MAX_CATALOG_ROWS} catalog rows"
            ));
        }
        let bytes: usize = self.trials.iter().map(NewTrial::payload_bytes).sum();
        if bytes > MAX_BATCH_BYTES {
            return Err(format!("a batch may hold at most {MAX_BATCH_BYTES} bytes"));
        }
        let mut seen = HashSet::with_capacity(self.trials.len());
        for t in &self.trials {
            t.check()?;
            if !seen.insert(t.seq) {
                return Err(format!("trial {} appears twice in one batch", t.seq));
            }
        }
        self.catalog.iter().try_for_each(CatalogRowRecord::check)
    }
}
