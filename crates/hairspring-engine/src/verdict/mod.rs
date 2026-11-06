//! The verdict ladder: how a trial's raw outcome becomes pass, finding, hang, inconclusive or infrastructure failure.
//!
//! Every backend that runs a program feeds this one ladder. A backend does not judge its trials. It declares the
//! facts it knows about one trial ([`TrialFacts`]: how the guest ended, whether it timed out, what its own tooling
//! reported, which lines and properties the guest produced) and the engine decides the verdict ([`judge`]).
//!
//! Two rules hold whatever the backend says. A trial that timed out is never a pass, and neither is one the engine
//! cannot classify: a pass needs a clean exit status and no unusable evidence.
mod facts;
mod ladder;
pub mod line_scan;
pub mod markers;
mod properties;
mod reported;

pub use facts::{DeadlockReport, TrialFacts};
pub use ladder::judge;
pub use line_scan::{LineRules, LineScan, Scanned};
pub use markers::{is_marker_line, marker_reason, LEGACY_VIOLATION_MARKER, VIOLATION_MARKER};
pub use properties::describe_failures;

/// What one trial means for the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Violation(String),
    /// The trial hit its time limit: possibly a hang or deadlock, possibly just slow. Not a finding by itself.
    Hang(String),
    /// The backend positively detected that nothing could run, and the guest exited non-zero. Unlike a timeout this
    /// is a finding without waiting for a wall-clock limit; it is never a data-property violation. The text carries
    /// the backend's own line.
    Deadlock(String),
    /// The backend or host failed, or the run is misconfigured: not a finding about the target, and the run cannot
    /// go on.
    Infra(String),
    /// This trial cannot be judged (the host failed to collect its events, or the guest's events are unusable) but
    /// the run can go on. Counted apart from passes and never a finding by itself.
    Inconclusive(String),
}

impl Verdict {
    /// A timeout is a finding only when the target says a hang is the bug. Every other verdict is unchanged.
    pub fn with_hang_policy(self, hang_is_violation: bool) -> Verdict {
        match self {
            Verdict::Hang(reason) if hang_is_violation => Verdict::Violation(reason),
            other => other,
        }
    }

    /// A trial cut off because the run ran out of budget, not because it ran out of patience: its hang or violation
    /// is inconclusive, never a finding. Every other verdict is unchanged.
    pub fn cut_by_budget(self) -> Verdict {
        match self {
            Verdict::Hang(reason) | Verdict::Violation(reason) => Verdict::Hang(reason),
            other => other,
        }
    }
}
