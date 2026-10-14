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
}
