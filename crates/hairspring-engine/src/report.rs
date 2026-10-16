use hairspring_props::{PropertyStat, RunSummary};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Passed,
    Failed,
    Error,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Passed => "passed",
            RunStatus::Failed => "failed",
            RunStatus::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub index: usize,
    pub action_id: String,
    pub label: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureReport {
    pub target: String,
    pub backend: String,
    pub message: String,
    pub original_len: usize,
    pub shrunk_len: usize,
    pub steps: Vec<Step>,
    /// Encoded `hairspring-core` bundle (see `Bundle::encode`).
    pub bundle: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub status: RunStatus,
    pub steps_executed: u64,
    pub trials: u64,
    /// Trials that ended in a timeout and were neither a pass nor a finding. Absent in older stored runs.
    #[serde(default)]
    pub inconclusive: u64,
    /// Distinct schedules the trials ran, when the backend can tell; 0 means unknown. Absent in older stored runs.
    #[serde(default)]
    pub distinct_schedules: u64,
    /// Work done across all trials (for Hermit: guest syscalls, including scheduler spins such as `sched_yield`); 0 means
    /// unknown. Absent in older stored runs.
    #[serde(default)]
    pub work: u64,
    pub elapsed_ms: u64,
    pub failure: Option<FailureReport>,
    pub error: Option<String>,
    /// Per-property statistics from the guest event protocol, sorted by id. Empty when the guest reported no
    /// properties, and absent in older stored runs. Not yet persisted by the store (issues #7 and #11).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyStat>,
    /// Run-wide counts behind `properties` (degraded, perturbing and report-missing trials). `None` without events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_run: Option<RunSummary>,
}

impl RunOutcome {
    /// Attaches the run's property statistics.
    pub fn with_properties(self, properties: Vec<PropertyStat>, run: RunSummary) -> Self {
        RunOutcome {
            property_run: (!properties.is_empty() || run.trials > 0).then_some(run),
            properties,
            ..self
        }
    }

    /// The outcome without its property statistics. Used where the statistics must not travel: a worker posting a
    /// result (thousands of ids would exceed the server's body limit) and the service receiving one (worker data is
    /// untrusted until the property checks of E6 exist). Remove its callers when E4 and E6 land.
    pub fn without_properties(self) -> Self {
        RunOutcome {
            properties: Vec::new(),
            property_run: None,
            ..self
        }
    }

    /// Properties that are must-hit and were never satisfied in any claimable trial. A run-level finding: it never
    /// fails an individual trial or turns a passing run into a failed one.
    pub fn unmet(&self) -> Vec<&PropertyStat> {
        self.properties
            .iter()
            .filter(|p| p.status == hairspring_props::PropertyStatus::Unmet)
            .collect()
    }

    pub fn with_inconclusive(self, inconclusive: u64) -> Self {
        RunOutcome {
            inconclusive,
            ..self
        }
    }

    pub fn with_schedule_stats(self, distinct_schedules: u64, work: u64) -> Self {
        RunOutcome {
            distinct_schedules,
            work,
            ..self
        }
    }

    pub fn passed(steps_executed: u64, trials: u64, elapsed_ms: u64) -> Self {
        RunOutcome {
            status: RunStatus::Passed,
            steps_executed,
            trials,
            inconclusive: 0,
            distinct_schedules: 0,
            work: 0,
            elapsed_ms,
            failure: None,
            error: None,
            properties: Vec::new(),
            property_run: None,
        }
    }
    pub fn failed(
        steps_executed: u64,
        trials: u64,
        elapsed_ms: u64,
        failure: FailureReport,
    ) -> Self {
        RunOutcome {
            status: RunStatus::Failed,
            steps_executed,
            trials,
            inconclusive: 0,
            distinct_schedules: 0,
            work: 0,
            elapsed_ms,
            failure: Some(failure),
            error: None,
            properties: Vec::new(),
            property_run: None,
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        RunOutcome {
            status: RunStatus::Error,
            steps_executed: 0,
            trials: 0,
            inconclusive: 0,
            distinct_schedules: 0,
            work: 0,
            elapsed_ms: 0,
            failure: None,
            error: Some(message.into()),
            properties: Vec::new(),
            property_run: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayOutcome {
    pub reproduced: bool,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorDivergence {
    pub seed: u64,
    pub step: usize,
}

/// Result of the determinism check: the same seeded walk run twice must agree step by step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub seeds_checked: u32,
    pub steps_per_seed: usize,
    pub divergence: Option<DoctorDivergence>,
}
