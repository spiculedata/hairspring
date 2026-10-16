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
    /// Ids declared with two different kinds (also reported as failures by [`TrialProps::verdict`]).
    pub conflicts: u64,
    /// `assert` lines dropped because their id has a kind conflict in the same process.
    pub dropped_asserts: u64,
    /// Ids a process catalogued but never reported although it sent `end`.
    pub unreported_ids: u64,
    /// Processes whose `end.sites` differs from the number of ids they catalogued.
    pub sites_mismatch: u64,
    /// Processes whose `end.threads` differs from the number of `thread` lines they sent.
    pub thread_count_mismatch: u64,
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
            + self.conflicts
            + self.dropped_asserts
            + self.unreported_ids
            + self.sites_mismatch
            + self.thread_count_mismatch
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
    /// The number of `thread` lines the process said it sent.
    pub threads: u64,
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
    /// `thread` lines in arrival order, unique by thread key.
    pub threads: Vec<ThreadReport>,
}

/// Counts of an `assert` line that was dropped because of a kind conflict, kept for diagnosis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DroppedCounts {
    pub hits: u64,
    pub trues: u64,
    pub falses: u64,
}

/// The same id declared with two different kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogConflict {
    /// Counts of the conflicting process's dropped `assert` line (zero if it sent none).
    pub dropped: DroppedCounts,
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
    /// The same id was declared with two different kinds. A trial-level error: counts for the id are unreliable.
    CatalogConflict,
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
    /// For a `CatalogConflict`: the counts that were dropped. `None` for other failures.
    pub dropped: Option<DroppedCounts>,
}

impl PropertyFailure {
    /// A one-line description naming the id, thread and ordinal, safe to print (strings were validated).
    pub fn describe(&self) -> String {
        let what = match self.reason {
            FailureReason::AlwaysFalse => "was false",
            FailureReason::UnreachableHit => "was reached",
            FailureReason::CatalogConflict => "has conflicting declarations",
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
        let dropped = self
            .dropped
            .map(|d| {
                format!(
                    " [dropped assert: hits {}, trues {}, falses {}]",
                    d.hits, d.trues, d.falses
                )
            })
            .unwrap_or_default();
        format!(
            "{} '{}' {} {} time(s){}{}{}",
            self.kind.as_str(),
            self.id,
            what,
            self.count,
            place,
            msg,
            dropped
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

    /// Per-thread reports (`proc`, report) from every process.
    pub fn thread_reports(&self) -> Vec<(&str, &ThreadReport)> {
        self.procs
            .iter()
            .flat_map(|p| p.threads.iter().map(move |t| (p.tag.as_str(), t)))
            .collect()
    }

    /// Trial-level failures, in id order, followed by catalog conflicts. Run-level UNMET is decided by
    /// [`crate::RunProps`].
    pub fn verdict(&self) -> Vec<PropertyFailure> {
        let conflicts = self.conflicts.iter().map(|c| self.conflict_failure(c));
        self.properties
            .iter()
            .filter_map(TrialProperty::failure)
            .chain(conflicts)
            .collect()
    }

    fn conflict_failure(&self, c: &CatalogConflict) -> PropertyFailure {
        let known = self.properties.iter().find(|p| p.id == c.id);
        PropertyFailure {
            id: c.id.clone(),
            kind: c.kept,
            reason: FailureReason::CatalogConflict,
            msg: format!(
                "also declared as {} by proc {}",
                c.rejected.as_str(),
                c.proc
            ),
            loc: known.and_then(|p| p.loc.clone()),
            count: 1,
            witness: None,
            dropped: Some(c.dropped),
        }
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
            dropped: None,
        })
    }
}
