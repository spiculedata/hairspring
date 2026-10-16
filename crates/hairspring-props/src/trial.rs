//! The folded view of one trial's event lines.

use crate::event::{AssertKind, EndReason, Loc, ThreadReport};

/// Counters for everything the parser or folder dropped, ignored or merged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventHealth {
    /// Event lines accepted into the fold.
    pub accepted: u64,
    /// Event-line bytes counted against the per-trial budget (newlines included).
    pub event_bytes: u64,
    /// Lines that claimed to be events but were malformed or inconsistent.
    pub malformed: u64,
    pub too_long: u64,
    pub invalid_utf8: u64,
    /// Lines dropped by a count or budget limit (ids, processes, event bytes).
    pub over_limit: u64,
    /// Repeated `hello`, `work`, `end` or per-id `assert` for one process (the first is kept).
    pub duplicates: u64,
    /// `assert` lines for an id the process never catalogued.
    pub uncatalogued: u64,
    /// Lines of an unknown kind (ignored).
    pub unknown_kind: u64,
    /// Lines with a newer major version (ignored).
    pub unsupported_version: u64,
    /// Unrecognised fields on accepted lines (ignored).
    pub unknown_fields: u64,
}

impl EventHealth {
    /// True when event data was lost or contradictory. Unknown kinds, versions and fields are additive and do not
    /// degrade a trial.
    pub fn degraded(&self) -> bool {
        self.malformed
            + self.too_long
            + self.invalid_utf8
            + self.over_limit
            + self.duplicates
            + self.uncatalogued
            > 0
    }
}

/// A witness located in a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Witness {
    pub proc: String,
    pub thread: String,
    /// The thread's own ordinal.
    pub ord: u64,
    pub vals: [i64; 4],
}

/// One id folded across the processes of a trial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialProperty {
    pub id: String,
    pub kind: AssertKind,
    pub must_hit: bool,
    pub msg: String,
    pub loc: Option<Loc>,
    pub hits: u64,
    pub trues: u64,
    pub falses: u64,
    pub first_false: Option<Witness>,
    pub first_true: Option<Witness>,
}

/// What `end` said about one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndReport {
    pub reason: EndReason,
    pub status: i64,
    pub sites: u64,
    pub evals: u64,
    pub dropped: u64,
    pub threads: Vec<ThreadReport>,
}

/// Everything one process reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcReport {
    pub tag: String,
    pub sdk: Option<String>,
    pub sdk_version: Option<String>,
    pub catalog_hash: Option<String>,
    pub signal_handlers: Option<bool>,
    pub work_ops: Option<u64>,
    pub end: Option<EndReport>,
}

/// The same id declared with two different kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogConflict {
    pub id: String,
    pub kept: AssertKind,
    pub rejected: AssertKind,
    pub proc: String,
}

/// A failure of a trial-level rule (a finding that gets a bundle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// An `always` or `always_or_unreachable` evaluated false.
    AlwaysFalse,
    /// An `unreachable` site was hit.
    UnreachableHit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyFailure {
    pub id: String,
    pub kind: AssertKind,
    pub reason: FailureReason,
    pub msg: String,
    pub loc: Option<Loc>,
    /// Number of false evaluations (`AlwaysFalse`) or hits (`UnreachableHit`).
    pub count: u64,
    pub witness: Option<Witness>,
}

impl PropertyFailure {
    /// A one-line description naming the id, thread and ordinal, safe to print (strings were validated).
    pub fn describe(&self) -> String {
        let what = match self.reason {
            FailureReason::AlwaysFalse => "was false",
            FailureReason::UnreachableHit => "was reached",
        };
        let place = self
            .witness
            .as_ref()
            .map(|w| format!(" (proc {}, thread {}, ordinal {})", w.proc, w.thread, w.ord))
            .unwrap_or_default();
        let msg = if self.msg.is_empty() {
            String::new()
        } else {
            format!(": {}", self.msg)
        };
        format!(
            "{} '{}' {} {} time(s){}{}",
            self.kind.as_str(),
            self.id,
            what,
            self.count,
            place,
            msg
        )
    }
}

/// One trial's properties, folded from its event lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrialProps {
    /// Processes in first-seen order.
    pub procs: Vec<ProcReport>,
    /// Properties sorted by id.
    pub properties: Vec<TrialProperty>,
    pub conflicts: Vec<CatalogConflict>,
    /// True when any per-evaluation `eval` line appeared (each is a syscall, so the schedule was perturbed).
    pub perturbing: bool,
    pub health: EventHealth,
}

impl TrialProps {
    pub fn is_degraded(&self) -> bool {
        self.health.degraded()
    }

    /// Process tags that never sent `end` (report lost, cut, or the process crashed with handlers off).
    pub fn report_missing(&self) -> Vec<&str> {
        self.procs
            .iter()
            .filter(|p| p.end.is_none())
            .map(|p| p.tag.as_str())
            .collect()
    }

    /// Guest-defined operation count summed over processes.
    pub fn total_work(&self) -> u64 {
        self.procs
            .iter()
            .filter_map(|p| p.work_ops)
            .fold(0u64, u64::saturating_add)
    }

    /// Per-thread reports (`proc`, report) from every process that sent `end`.
    pub fn thread_reports(&self) -> Vec<(&str, &ThreadReport)> {
        self.procs
            .iter()
            .filter_map(|p| p.end.as_ref().map(|e| (p.tag.as_str(), e)))
            .flat_map(|(tag, e)| e.threads.iter().map(move |t| (tag, t)))
            .collect()
    }

    /// Trial-level failures, in id order. Run-level UNMET is decided by [`crate::RunProps`].
    pub fn verdict(&self) -> Vec<PropertyFailure> {
        self.properties
            .iter()
            .filter_map(TrialProperty::failure)
            .collect()
    }
}

impl TrialProperty {
    /// The trial-level failure of this property, if any.
    pub fn failure(&self) -> Option<PropertyFailure> {
        let p = self;
        let (reason, count, witness) = match p.kind {
            AssertKind::Always | AssertKind::AlwaysOrUnreachable if p.falses > 0 => {
                (FailureReason::AlwaysFalse, p.falses, p.first_false.clone())
            }
            AssertKind::Unreachable if p.hits > 0 => (
                FailureReason::UnreachableHit,
                p.hits,
                p.first_true.clone().or_else(|| p.first_false.clone()),
            ),
            _ => return None,
        };
        Some(PropertyFailure {
            id: p.id.clone(),
            kind: p.kind,
            reason,
            msg: p.msg.clone(),
            loc: p.loc.clone(),
            count,
            witness,
        })
    }
}
