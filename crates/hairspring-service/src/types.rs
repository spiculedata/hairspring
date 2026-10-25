use crate::RunCompletion;
use hairspring_engine::config::Budget;
use hairspring_engine::report::Step;
use hairspring_store::{Contributor, RunRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRun {
    pub target: String,
    pub seed: Option<u64>,
    pub budget: Option<Budget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JobPayload {
    pub target: String,
    pub seed: u64,
    pub budget: Option<Budget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetInfo {
    pub name: String,
    pub kind: String,
    pub description: String,
    pub budget: Budget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureSummary {
    pub id: String,
    pub run_id: String,
    pub target: String,
    pub message: String,
    pub original_len: usize,
    pub shrunk_len: usize,
    pub bundle_hash: String,
    pub created_at: i64,
    /// Where the failure's stored confirmation checks leave it. Present on a run's failures, left out of lists of
    /// failures across runs, and absent from a summary stored before checks existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding: Option<crate::FindingSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunDetail {
    pub run: RunRecord,
    pub failures: Vec<FailureSummary>,
    /// Property figures, for runs whose trials were stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<PropertySummary>,
}

/// Text that comes from the program under test. Anything that renders it, or hands it to a model, must treat it as
/// untrusted.
#[derive(Debug, Clone, Serialize)]
pub struct GuestStrings {
    pub note: String,
    pub violation_ids: Vec<String>,
    pub unmet_ids: Vec<String>,
}

/// A run's properties in a few lines.
#[derive(Debug, Clone, Serialize)]
pub struct PropertySummary {
    pub catalog_size: u64,
    /// Number of properties by status: pass, violation, unmet, unrated.
    pub counts: BTreeMap<String, u64>,
    /// The lowest failing seed among judged trials, as a decimal string (a seed can exceed 2^53).
    pub first_failing_seed: Option<String>,
    /// The run stopped at its first confirmed failure, so its trial count is not a fixed budget.
    pub stopped_at_first_failure: bool,
    /// The trial rows held do not match what the worker said it ran: no must-hit claim is made.
    pub ingest_incomplete: bool,
    /// Some trial came from a worker that is not trusted.
    pub unverified_worker_data: bool,
    /// How the run ended against its trial budget.
    pub completion: RunCompletion,
    /// The wording for `completion`.
    pub completion_label: String,
    pub guest: GuestStrings,
}

/// What a batch of trial records did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestReceipt {
    pub inserted: u32,
    pub duplicates: u32,
}

/// Who reported a run's trials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunTrust {
    pub workers: Vec<Contributor>,
    /// Every trial came from a worker that was trusted when it reported.
    pub all_trusted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailureDetail {
    pub record: FailureSummary,
    pub steps: Vec<Step>,
    pub bundle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeasedJob {
    pub job_id: String,
    pub run_id: String,
    pub target: String,
    pub seed: u64,
    pub budget: Option<Budget>,
}

/// Access level of a caller. Higher roles include every lower one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Read,
    Run,
    Admin,
}

impl Role {
    pub fn allows(self, needed: Role) -> bool {
        self >= needed
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s.to_ascii_lowercase().as_str() {
            "read" => Some(Role::Read),
            "run" => Some(Role::Run),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Read => "read",
            Role::Run => "run",
            Role::Admin => "admin",
        }
    }
}
