use anthill_engine::config::Budget;
use anthill_engine::report::Step;
use anthill_store::RunRecord;
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone, Serialize)]
pub struct RunDetail {
    pub run: RunRecord,
    pub failures: Vec<FailureSummary>,
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
