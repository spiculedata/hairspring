use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug)]
pub enum StoreError {
    NotFound(String),
    Backend(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::NotFound(w) => write!(f, "not found: {w}"),
            StoreError::Backend(m) => write!(f, "storage error: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

pub type Result<T> = std::result::Result<T, StoreError>;

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "lowercase")]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn as_str(&self) -> &'static str { match self { $($name::$variant => $text),+ } }
            pub fn parse(s: &str) -> Option<$name> { match s { $($text => Some($name::$variant)),+, _ => None } }
        }
    };
}

string_enum!(RunState { Queued => "queued", Running => "running", Passed => "passed", Failed => "failed", Error => "error", Cancelled => "cancelled" });
string_enum!(JobStatus { Queued => "queued", Running => "running", Done => "done", Failed => "failed" });

impl RunState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RunState::Passed | RunState::Failed | RunState::Error | RunState::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun {
    pub target: String,
    pub backend: String,
    pub seed: u64,
    pub config_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub target: String,
    pub backend: String,
    pub state: RunState,
    pub seed: u64,
    pub config_json: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub steps: u64,
    pub trials: u64,
    pub error: Option<String>,
    /// Trials that timed out or could not be judged. Zero for runs stored before schema version 2.
    pub inconclusive: u64,
    /// Distinct schedules the trials ran; zero when unknown.
    pub distinct_schedules: u64,
    /// Work done across all trials; zero when unknown.
    pub work: u64,
    /// How the run chose its trials: `search` (stop at the first confirmed failure) is the only mode so far.
    pub mode: String,
    /// The run stopped at its first confirmed failure, so its trial count is not a fixed budget.
    pub stopped_at_first_failure: bool,
    /// The folded `RunSummary` as JSON, written by the server when the run finishes.
    pub property_summary_json: Option<String>,
    /// Ids in the run catalog.
    pub catalog_size: u64,
    /// The trial rows held do not match what the worker said it ran, so no must-hit claim is made from them.
    pub ingest_incomplete: bool,
    pub build_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFailure {
    pub run_id: String,
    pub target: String,
    pub message: String,
    pub original_len: usize,
    pub shrunk_len: usize,
    pub bundle_hash: String,
    pub steps_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureRecord {
    pub id: String,
    pub run_id: String,
    pub target: String,
    pub message: String,
    pub original_len: usize,
    pub shrunk_len: usize,
    pub bundle_hash: String,
    pub steps_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub run_id: String,
    pub kind: String,
    pub payload_json: String,
    pub status: JobStatus,
    pub lease_owner: Option<String>,
    pub lease_until: Option<i64>,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: i64,
    pub run_id: String,
    pub ts: i64,
    pub kind: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRecord {
    pub id: String,
    pub name: String,
    pub caps_json: String,
    pub registered_at: i64,
    pub last_seen: i64,
    /// Set by an administrator. Claims need trusted workers; every other view shows untrusted data, labelled.
    pub trusted: bool,
}

/// A job whose lease expired after its last permitted attempt; its run must be finalised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredJob {
    pub job_id: String,
    pub run_id: String,
}

/// Everything recorded atomically when a worker finishes a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobResult {
    pub state: RunState,
    pub steps: u64,
    pub trials: u64,
    pub error: Option<String>,
    /// Attached to the job's own run; `NewFailure::run_id` is ignored.
    pub failure: Option<NewFailure>,
    /// Payload of the `finished` event.
    pub event_payload_json: String,
    /// Run-level figures and property rows, written in the same transaction.
    pub stats: crate::trials::RunStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishOutcome {
    /// The caller does not hold the lease (never did, lost it, or already finished).
    NotLeased,
    /// The result was recorded.
    Applied,
    /// The lease was held but the run was already final (for example cancelled); the job is closed, nothing else changed.
    RunAlreadyFinal,
}
