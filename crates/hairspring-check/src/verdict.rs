//! Verdicts. A checker answers `Ok`, `Violation` (with a witness) or `Unknown`; `Unknown` is never a pass.

use crate::history::Operation;

/// Version of the checkers' semantics. Bump when a checker's answer for the same input can change (it belongs in the
/// run identity next to [`CheckerId::as_str`]).
pub const CHECKER_VERSION: u32 = 1;

/// Which checker produced a verdict. The names are the configuration names of the checkers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CheckerId {
    /// No acknowledged add is missing from the final set, and nothing appears that was not added.
    SetNoLostAcks,
    /// No acknowledged enqueue is lost, nothing is delivered twice, nothing is delivered that was not enqueued.
    QueueNoLossNoDup,
    /// The recovered log is a prefix of the attempted appends and holds every acknowledged one.
    WalPrefix,
    /// Every key's operations can be ordered as a legal single-register history that respects real time.
    RegisterLinearizable,
}

impl CheckerId {
    /// Stable identifier used in configuration and reports.
    pub fn as_str(self) -> &'static str {
        match self {
            CheckerId::SetNoLostAcks => "set-no-lost-acks",
            CheckerId::QueueNoLossNoDup => "queue-no-loss-no-dup",
            CheckerId::WalPrefix => "wal-prefix",
            CheckerId::RegisterLinearizable => "register-linearizable",
        }
    }
}

/// What went wrong, with the values involved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViolationKind {
    /// An acknowledged write is missing from the final state.
    LostAcknowledgedWrite { value: i64 },
    /// An acknowledged enqueue was neither delivered nor left in the queue.
    LostMessage { value: i64 },
    /// A value appears in the final state, or was delivered, that no non-failed operation could have produced.
    Phantom { value: i64 },
    /// A value was delivered or recorded more often than non-failed operations could have produced it.
    Duplicate { value: i64, seen: u64, at_most: u64 },
    /// The recovered log is not a prefix of the attempted appends: `index` is the first differing position.
    NotPrefix { index: usize },
    /// The recovered log is shorter than the acknowledged appends require.
    TruncatedBelowAcknowledged { recovered: usize, required: usize },
    /// No legal order of the operations respects real time.
    NotLinearizable,
}

/// A conclusive finding. `witness` holds the operations that show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub checker: CheckerId,
    /// The key the violation is on.
    pub key: String,
    pub kind: ViolationKind,
    /// The shortest sub-history the checker could show, ordered by invocation. For linearizability this is
    /// 1-minimal (removing any one operation makes it linearizable) when `witness_minimal` is true.
    pub witness: Vec<Operation>,
    /// False when the witness search ran out of its own budget: the witness still shows the violation but may
    /// contain operations that are not needed.
    pub witness_minimal: bool,
}

/// Why a checker could not decide. None of these is a pass and none is a failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnknownReason {
    /// The history is flagged incomplete (a client died, or lines were dropped), so absence proves nothing.
    IncompleteHistory,
    /// The linearizability search used its step budget.
    StepBudgetExhausted { max_steps: u64 },
    /// The linearizability search used its memo budget.
    MemoBudgetExhausted { max_entries: usize },
    /// A key has more operations than the limit allows.
    TooManyOperations {
        key: String,
        operations: usize,
        max: usize,
    },
    /// An operation this checker does not define (unknown function name, or a missing argument).
    UnsupportedOperation { key: String, f: String },
    /// A shape the checker does not model, for example overlapping appends on one write-ahead log.
    UnsupportedShape { key: String, what: &'static str },
}

/// The answer of a checker.
#[must_use = "a verdict must be reported; Unknown is not a pass"]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Decided: no violation.
    Ok,
    /// Decided: a violation, with a witness.
    Violation(Box<Violation>),
    /// Not decided. Counted as inconclusive, never as a pass.
    Unknown(UnknownReason),
}

impl Verdict {
    /// True only for a decided `Ok`. `Unknown` is not a pass.
    pub fn is_pass(&self) -> bool {
        matches!(self, Verdict::Ok)
    }

    /// True for `Ok` and `Violation`; false for `Unknown`.
    pub fn is_conclusive(&self) -> bool {
        !matches!(self, Verdict::Unknown(_))
    }

    /// The violation, when there is one.
    pub fn violation(&self) -> Option<&Violation> {
        match self {
            Verdict::Violation(v) => Some(v),
            _ => None,
        }
    }
}
