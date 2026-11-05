//! Retention: deleting old runs and what only they refer to, the same way on every backend.
//!
//! The store never decides what is "old enough" on its own authority: the caller names the runs and a cutoff, and the
//! store re-checks, inside the transaction that deletes, that each run is finished, has no job that could still run,
//! and is older than the cutoff. A run that fails any check is kept and counted under the reason.
//!
//! A trial tree is one unit. Its root run and every branch run are deleted together, in one transaction, and only
//! when every one of them passes the checks and was named by the caller; otherwise the whole tree is kept. Within a
//! unit the order is the order the foreign keys need: check results, tree nodes, branches, the tree, then each run's
//! own rows, then the run. Blobs (bundles, stored output, tree edges) are content addressed and shared, so a blob is
//! removed only when nothing that remains refers to it.
//!
//! A dry run does the same work inside one transaction and rolls it back, so its counts are exactly what an apply of
//! the same runs would have removed at that moment.
use crate::types::{Result, RunState};
use std::collections::{BTreeMap, BTreeSet};

/// A run old enough to be looked at, with the time its age is measured from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionCandidate {
    pub id: String,
    pub state: RunState,
    /// When the run finished, or when it was created if it never finished.
    pub aged_from: i64,
}

/// Why a run was not deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeptReason {
    /// Queued or running.
    Active,
    /// Finished, but a job of the run is queued or running.
    ActiveJob,
    /// Not older than the cutoff.
    TooNew,
    /// Part of a trial tree that has a run that is kept (or that the caller did not name).
    TreeKept,
    /// No such run.
    NotFound,
}

impl KeptReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            KeptReason::Active => "active",
            KeptReason::ActiveJob => "active_job",
            KeptReason::TooNew => "too_new",
            KeptReason::TreeKept => "tree_kept",
            KeptReason::NotFound => "not_found",
        }
    }
}

/// What a retention pass removed (or, for a dry run, would remove).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionReport {
    /// False for a dry run: nothing was changed.
    pub applied: bool,
    pub run_ids: Vec<String>,
    /// Deleted runs by state.
    pub runs_by_state: BTreeMap<String, u64>,
    pub failures: u64,
    pub checks: u64,
    pub trials: u64,
    pub events: u64,
    pub jobs: u64,
    pub trees_deleted: Vec<String>,
    pub trees_kept: Vec<String>,
    /// Tree nodes deleted.
    pub nodes: u64,
    /// Blobs removed: ones nothing that remains refers to. `blob_bytes` is their stored size.
    pub blobs: u64,
    pub blob_bytes: u64,
    /// The removed blobs that were failure bundles (also counted in `blobs`), and their hashes.
    pub bundle_blobs: u64,
    pub bundle_bytes: u64,
    pub bundle_hashes: Vec<String>,
    /// Runs kept, by reason.
    pub kept: BTreeMap<&'static str, u64>,
    /// Set when a unit failed to delete and the pass stopped: the report holds what had been committed before it.
    pub error: Option<String>,
}

impl RetentionReport {
    pub fn runs_deleted(&self) -> u64 {
        self.run_ids.len() as u64
    }

    /// Adds what `other` removed or kept to this report (not `applied` or `error`).
    pub(crate) fn absorb(&mut self, other: RetentionReport) {
        self.run_ids.extend(other.run_ids);
        for (k, v) in other.runs_by_state {
            *self.runs_by_state.entry(k).or_insert(0) += v;
        }
        self.failures += other.failures;
        self.checks += other.checks;
        self.trials += other.trials;
        self.events += other.events;
        self.jobs += other.jobs;
        self.trees_deleted.extend(other.trees_deleted);
        self.trees_kept.extend(other.trees_kept);
        self.nodes += other.nodes;
        self.blobs += other.blobs;
        self.blob_bytes += other.blob_bytes;
        self.bundle_blobs += other.bundle_blobs;
        self.bundle_bytes += other.bundle_bytes;
        self.bundle_hashes.extend(other.bundle_hashes);
        for (k, v) in other.kept {
            *self.kept.entry(k).or_insert(0) += v;
        }
    }

    fn keep(&mut self, reason: KeptReason, runs: u64) {
        *self.kept.entry(reason.as_str()).or_insert(0) += runs;
    }
}

/// The facts the checks need about one run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunFacts {
    pub state: RunState,
    pub aged_from: i64,
    pub has_active_job: bool,
}

/// What deleting one unit removed, before blobs.
#[derive(Debug, Default)]
pub(crate) struct UnitRemoved {
    pub failures: u64,
    pub checks: u64,
    pub trials: u64,
    pub events: u64,
    pub jobs: u64,
    pub nodes: u64,
    pub blobs: Vec<RemovedBlob>,
}

#[derive(Debug)]
pub(crate) struct RemovedBlob {
    pub hash: String,
    pub bytes: u64,
    pub is_bundle: bool,
}

/// The backend half: reads and deletes inside one open transaction.
pub(crate) trait RetentionOps {
    /// `None` for an unknown run. Backends lock what they read where the database needs it.
    fn run_facts(&mut self, id: &str) -> Result<Option<RunFacts>>;
    /// The tree the run belongs to: the one it roots or the one it branches from.
    fn tree_of_run(&mut self, id: &str) -> Result<Option<String>>;
    /// The runs of a tree: its root and every branch run, whichever tables say so.
    fn tree_members(&mut self, tree_id: &str) -> Result<Vec<String>>;
    /// Deletes the runs (and the tree, when given) and every row only they own, and then the blobs nothing refers to.
    fn delete_unit(&mut self, tree_id: Option<&str>, runs: &[String]) -> Result<UnitRemoved>;
}

/// Why `facts` keeps a run, or `None` when it may go.
fn check(facts: Option<RunFacts>, cutoff_ms: i64) -> Option<KeptReason> {
    let Some(f) = facts else {
        return Some(KeptReason::NotFound);
    };
    if !f.state.is_terminal() {
        return Some(KeptReason::Active);
    }
    if f.has_active_job {
        return Some(KeptReason::ActiveJob);
    }
    if f.aged_from >= cutoff_ms {
        return Some(KeptReason::TooNew);
    }
    None
}

/// Looks at the unit `anchor` belongs to and deletes it if every member passes. Members already dealt with are
/// recorded in `done`, so each run is judged once per pass. Called inside the transaction that deletes.
pub(crate) fn process_anchor(
    ops: &mut dyn RetentionOps,
    anchor: &str,
    named: &BTreeSet<&str>,
    cutoff_ms: i64,
    done: &mut BTreeSet<String>,
    report: &mut RetentionReport,
) -> Result<()> {
    if done.contains(anchor) {
        return Ok(());
    }
    let tree = ops.tree_of_run(anchor)?;
    let members = match &tree {
        Some(t) => ops.tree_members(t)?,
        None => vec![anchor.to_owned()],
    };
    done.extend(members.iter().cloned());
    let mut states = Vec::new();
    let mut reasons = Vec::new();
    for id in &members {
        if named.contains(id.as_str()) {
            let facts = ops.run_facts(id)?;
            states.push(facts.map(|f| f.state));
            reasons.push(check(facts, cutoff_ms));
        } else {
            states.push(None);
            reasons.push(Some(KeptReason::TreeKept));
        }
    }
    if reasons.iter().any(Option::is_some) {
        keep_unit(report, &tree, &members, named, &reasons);
        return Ok(());
    }
    let removed = ops.delete_unit(tree.as_deref(), &members)?;
    record_removed(report, &tree, &members, &states, removed);
    Ok(())
}

/// Counts a kept unit. A tree with a kept run keeps all its runs: each run the caller named is counted under its own
/// reason, or as `tree_kept` when it would have gone on its own. Runs the caller did not name were never candidates.
fn keep_unit(
    report: &mut RetentionReport,
    tree: &Option<String>,
    members: &[String],
    named: &BTreeSet<&str>,
    reasons: &[Option<KeptReason>],
) {
    if let Some(t) = tree {
        report.trees_kept.push(t.clone());
    }
    for (id, reason) in members.iter().zip(reasons) {
        if named.contains(id.as_str()) {
            report.keep(reason.unwrap_or(KeptReason::TreeKept), 1);
        }
    }
}

fn record_removed(
    report: &mut RetentionReport,
    tree: &Option<String>,
    members: &[String],
    states: &[Option<RunState>],
    removed: UnitRemoved,
) {
    for (id, state) in members.iter().zip(states) {
        report.run_ids.push(id.clone());
        let name = state.map_or("unknown", |s| s.as_str());
        *report.runs_by_state.entry(name.to_owned()).or_insert(0) += 1;
    }
    if let Some(t) = tree {
        report.trees_deleted.push(t.clone());
    }
    report.failures += removed.failures;
    report.checks += removed.checks;
    report.trials += removed.trials;
    report.events += removed.events;
    report.jobs += removed.jobs;
    report.nodes += removed.nodes;
    for blob in removed.blobs {
        report.blobs += 1;
        report.blob_bytes += blob.bytes;
        if blob.is_bundle {
            report.bundle_blobs += 1;
            report.bundle_bytes += blob.bytes;
            report.bundle_hashes.push(blob.hash);
        }
    }
}

/// The ids to work through: deduplicated, in a fixed order.
pub(crate) fn anchors(run_ids: &[String]) -> Vec<&str> {
    let set: BTreeSet<&str> = run_ids.iter().map(String::as_str).collect();
    set.into_iter().collect()
}
