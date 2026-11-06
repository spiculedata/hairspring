//! What a backend tells the engine about one finished trial.
use hairspring_props::PropertyFailure;

/// A deadlock the backend positively detected, as it reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadlockReport {
    /// Who reported it, for the verdict text (the backend's own name for its tooling).
    pub reporter: String,
    /// The report line, trimmed.
    pub line: String,
}

/// The facts one backend declares about one trial. Absent facts are the default (nothing happened). The engine
/// weighs them in a fixed order; the backend decides only what each fact says, never what the verdict is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrialFacts {
    /// The limit the trial ran under, for the hang text.
    pub timeout_secs: u64,
    pub timed_out: bool,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// The backend's own tooling failed, or refused its command line. Not about the guest: the run cannot go on.
    /// The text is the whole message.
    pub backend_failure: Option<String>,
    /// The configuration of the guest's event channel is wrong: the operator's to fix, and the run cannot go on.
    pub config_fault: Option<String>,
    /// The host could not collect the guest's events (a full scratch disk). The trial is inconclusive. The text is
    /// the problem alone.
    pub collection_fault: Option<String>,
    /// Failing properties the guest reported, sorted. A false that was read counts even when the rest of the report
    /// is damaged, so these are weighed before the exit status.
    pub property_failures: Vec<PropertyFailure>,
    /// The first line of the guest's output that starts with a violation marker, from stdout and then stderr.
    pub marker_line: Option<String>,
    pub deadlock: Option<DeadlockReport>,
    /// Why a trial that exited cleanly still cannot be called a pass: its event data was unusable or incomplete.
    pub unusable_evidence: Option<String>,
}
