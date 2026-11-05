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
//! An apply works in batches: whole units, up to [`BATCH_RUNS`] runs judged, in one transaction. Rows are deleted
//! unit by unit; the blobs the batch's runs referred to are then checked together, with one pass over each table that
//! can refer to a blob, however many blobs there are, and only that last step holds the blob lock where the backend
//! needs one. Without an index on the referring columns each pass still reads the whole `trials` table, so a batch
//! costs O(trials); batching keeps the number of passes to one per batch rather than one per blob.
//!
//! A dry run takes no lock that a writer waits for: it reads one consistent snapshot, counts what each unit would
//! remove, and works out the blobs nothing outside the removed runs refers to. It is an estimate as of that snapshot;
//! an apply checks everything again under its locks.
use crate::types::{Result, RunState, StoreError};
use std::collections::{BTreeMap, BTreeSet};

/// The most runs an apply judges in one transaction (a tree is never split, so a batch can hold more). Small enough
/// that a batch's locks are short, large enough that the reference pass is not repeated for every run.
pub(crate) const BATCH_RUNS: usize = 20;

/// The most candidate blobs checked in one reference pass. An apply's batch rarely has more; a dry run over a large
/// store can have hundreds of thousands, and checking them all in one statement can need more memory than the
/// database allows a query (a 100000-hash pass ran out of PostgreSQL's shared memory in a default container).
const REFERENCE_CHUNK: usize = 5_000;

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

/// Proof that this process holds the store's retention lock; dropping it releases the lock.
pub struct RetentionLock {
    _held: Box<dyn Send>,
}

impl RetentionLock {
    pub(crate) fn new(held: Box<dyn Send>) -> RetentionLock {
        RetentionLock { _held: held }
    }
}

impl std::fmt::Debug for RetentionLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RetentionLock")
    }
}

/// The facts the checks need about one run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunFacts {
    pub state: RunState,
    pub aged_from: i64,
    pub has_active_job: bool,
}

/// What deleting one unit removed, before blobs, and the blobs its rows referred to (hash, and whether it is a failure
/// bundle): the candidates for removal once the batch is done.
#[derive(Debug, Default)]
pub(crate) struct UnitRemoved {
    pub failures: u64,
    pub checks: u64,
    pub trials: u64,
    pub events: u64,
    pub jobs: u64,
    pub nodes: u64,
    pub blobs: BTreeMap<String, bool>,
}

/// The backend half: reads and deletes inside one open transaction.
pub(crate) trait RetentionOps {
    /// `None` for an unknown run. Backends lock what they read where the database needs it.
    fn run_facts(&mut self, id: &str) -> Result<Option<RunFacts>>;
    /// The tree the run belongs to: the one it roots or the one it branches from.
    fn tree_of_run(&mut self, id: &str) -> Result<Option<String>>;
    /// Locks the tree against new branches and nodes until the transaction ends, where the database needs it, so the
    /// members read next stay the members. False when the tree no longer exists.
    fn lock_tree(&mut self, tree_id: &str) -> Result<bool>;
    /// The runs of a tree: its root and every branch run, whichever tables say so.
    fn tree_members(&mut self, tree_id: &str) -> Result<Vec<String>>;
    /// Deletes the runs (and the tree, when given) and every row only they own, and returns the blobs those rows
    /// referred to; a dry run counts the rows instead and remembers the runs as removed. Tree rows are deleted only for
    /// the listed runs; if the tree still has rows for any other run, it returns an error and the caller's transaction
    /// is rolled back, so nothing of the unit is removed.
    fn delete_unit(&mut self, tree_id: Option<&str>, runs: &[String]) -> Result<UnitRemoved>;
    /// Waits for every transaction that is storing a blob and keeps new ones out until this one ends, where the
    /// database needs it. A dry run takes nothing.
    fn lock_blobs(&mut self) -> Result<()>;
    /// Of `candidates`, the hashes that something which remains still refers to: a trial row, failure or tree node
    /// that names the hash, or a stored check whose artefact list contains it. One pass over each table. Rows of the
    /// runs a dry run counted as removed do not count.
    fn referenced(&mut self, candidates: &[String]) -> Result<BTreeSet<String>>;
    /// Deletes the blobs (a dry run only reads them) and returns the hash and stored size of each that existed.
    fn remove_blobs(&mut self, hashes: &[String]) -> Result<Vec<(String, u64)>>;
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

/// The fixed inputs of a pass: the runs the caller named and the cutoff.
pub(crate) struct Pass<'a> {
    pub named: BTreeSet<&'a str>,
    pub cutoff_ms: i64,
}

/// Works through `anchors` from `*next` inside one open transaction, until at least `max_runs` runs have been judged
/// (whole units only) or none are left, then removes the blobs nothing that remains refers to. `*next` is left at the
/// first anchor not looked at. On an error the caller rolls the transaction back and discards `batch`.
pub(crate) fn process_batch(
    ops: &mut dyn RetentionOps,
    pass: &Pass,
    anchors: &[&str],
    next: &mut usize,
    max_runs: usize,
    done: &mut BTreeSet<String>,
    batch: &mut RetentionReport,
) -> Result<()> {
    let mut candidates: BTreeMap<String, bool> = BTreeMap::new();
    while *next < anchors.len() && judged(batch) < max_runs {
        let anchor = anchors[*next];
        *next += 1;
        process_anchor(ops, anchor, pass, done, batch, &mut candidates)?;
    }
    remove_unreferenced(ops, candidates, batch)
}

/// Runs a batch has judged: removed or kept.
fn judged(r: &RetentionReport) -> usize {
    r.run_ids.len() + r.kept.values().sum::<u64>() as usize
}

/// Removes the candidates nothing that remains refers to, checked all at once under the blob lock.
fn remove_unreferenced(
    ops: &mut dyn RetentionOps,
    candidates: BTreeMap<String, bool>,
    report: &mut RetentionReport,
) -> Result<()> {
    if candidates.is_empty() {
        return Ok(());
    }
    ops.lock_blobs()?;
    let hashes: Vec<String> = candidates.keys().cloned().collect();
    let mut still = BTreeSet::new();
    for chunk in hashes.chunks(REFERENCE_CHUNK) {
        still.extend(ops.referenced(chunk)?);
    }
    let unreferenced: Vec<String> = hashes.into_iter().filter(|h| !still.contains(h)).collect();
    for (hash, bytes) in ops.remove_blobs(&unreferenced)? {
        report.blobs += 1;
        report.blob_bytes += bytes;
        if candidates.get(&hash).copied().unwrap_or(false) {
            report.bundle_blobs += 1;
            report.bundle_bytes += bytes;
            report.bundle_hashes.push(hash);
        }
    }
    Ok(())
}

/// The candidates that occur anywhere in `texts`, as plain substrings (what a stored check's artefact list is searched
/// for). Each text is read once: every window of each candidate length is looked up in a set, so the cost is the
/// length of the texts, not the texts times the candidates.
pub(crate) fn named_in_texts<I>(candidates: &[String], texts: I) -> Result<BTreeSet<String>>
where
    I: IntoIterator<Item = Result<String>>,
{
    let wanted: BTreeSet<&[u8]> = candidates
        .iter()
        .filter(|c| !c.is_empty())
        .map(|c| c.as_bytes())
        .collect();
    let lengths: BTreeSet<usize> = wanted.iter().map(|c| c.len()).collect();
    let mut found = BTreeSet::new();
    for text in texts {
        let text = text?;
        for &len in &lengths {
            for window in text.as_bytes().windows(len) {
                if wanted.contains(window) {
                    found.insert(String::from_utf8_lossy(window).into_owned());
                }
            }
        }
        if found.len() == wanted.len() {
            break;
        }
    }
    Ok(found)
}

/// Looks at the unit `anchor` belongs to and deletes it if every member passes. Members already dealt with are
/// recorded in `done`, so each run is judged once per pass. The blobs the unit's rows referred to are added to
/// `candidates`. Called inside the transaction that deletes.
fn process_anchor(
    ops: &mut dyn RetentionOps,
    anchor: &str,
    pass: &Pass,
    done: &mut BTreeSet<String>,
    report: &mut RetentionReport,
    candidates: &mut BTreeMap<String, bool>,
) -> Result<()> {
    let (named, cutoff_ms) = (&pass.named, pass.cutoff_ms);
    if done.contains(anchor) {
        return Ok(());
    }
    let mut tree = ops.tree_of_run(anchor)?;
    // Lock the tree before reading its members: a branch added after the read would otherwise be deleted with the
    // tree although it was never judged. A tree that is gone (another pass removed it) leaves the run on its own.
    if let Some(t) = &tree {
        if !ops.lock_tree(t)? {
            tree = None;
        }
    }
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
    // A run judged on its own may have joined a tree since it was first read. The run is locked now (run_facts), so
    // that cannot happen any more; ask again.
    if tree.is_none() && reasons.iter().all(Option::is_none) && ops.tree_of_run(anchor)?.is_some() {
        reasons = vec![Some(KeptReason::TreeKept)];
    }
    if reasons.iter().any(Option::is_some) {
        keep_unit(report, &tree, &members, named, &reasons);
        return Ok(());
    }
    let mut removed = ops.delete_unit(tree.as_deref(), &members)?;
    for (hash, is_bundle) in std::mem::take(&mut removed.blobs) {
        *candidates.entry(hash).or_insert(false) |= is_bundle;
    }
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
}

/// The error for a tree that still has rows for runs outside the unit being removed. It should never happen (the
/// tree is locked, or the database written by one writer at a time, before its members are read); if it does, the
/// unit's transaction is rolled back and nothing of the unit is removed.
pub(crate) fn tree_changed(tree: &str) -> StoreError {
    StoreError::Backend(format!(
        "trial tree {tree} has rows for runs that were not judged with it; nothing of the tree was removed"
    ))
}

/// The ids to work through: deduplicated, in a fixed order.
pub(crate) fn anchors(run_ids: &[String]) -> Vec<&str> {
    let set: BTreeSet<&str> = run_ids.iter().map(String::as_str).collect();
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(candidates: &[&str], texts: &[&str]) -> Vec<String> {
        let candidates: Vec<String> = candidates.iter().map(|c| (*c).to_owned()).collect();
        named_in_texts(&candidates, texts.iter().map(|t| Ok((*t).to_owned())))
            .unwrap()
            .into_iter()
            .collect()
    }

    #[test]
    fn a_candidate_counts_wherever_it_appears_as_text() {
        assert_eq!(
            found(&["abcd", "cdef", "zz"], &["xxabcdefxx"]),
            ["abcd", "cdef"],
            "overlapping occurrences both count"
        );
        assert_eq!(found(&["ab", "abc"], &["[\"abc\"]"]), ["ab", "abc"]);
        assert_eq!(found(&["abc"], &["ab", "c"]), Vec::<String>::new());
        assert_eq!(found(&["abc"], &["", "abc"]), ["abc"]);
        assert_eq!(
            found(&["ab", "cd"], &["ab", "xx", "cd"]),
            ["ab", "cd"],
            "every text is read until all are found"
        );
        assert_eq!(found(&[""], &["anything"]), Vec::<String>::new());
    }

    /// A backend in which every run is old and finished, owns `blobs_per_run` blobs, and every blob is still referred
    /// to. It records the size of each reference check.
    struct AllReferenced {
        blobs_per_run: usize,
        checks: Vec<usize>,
        lock_taken_before_check: bool,
        locked: bool,
    }

    impl RetentionOps for AllReferenced {
        fn run_facts(&mut self, _: &str) -> crate::types::Result<Option<RunFacts>> {
            Ok(Some(RunFacts {
                state: crate::types::RunState::Passed,
                aged_from: 0,
                has_active_job: false,
            }))
        }
        fn tree_of_run(&mut self, _: &str) -> crate::types::Result<Option<String>> {
            Ok(None)
        }
        fn lock_tree(&mut self, _: &str) -> crate::types::Result<bool> {
            Ok(true)
        }
        fn tree_members(&mut self, _: &str) -> crate::types::Result<Vec<String>> {
            Ok(Vec::new())
        }
        fn delete_unit(
            &mut self,
            _: Option<&str>,
            runs: &[String],
        ) -> crate::types::Result<UnitRemoved> {
            let blobs = (0..self.blobs_per_run)
                .map(|i| (format!("{}-{i}", runs[0]), false))
                .collect();
            Ok(UnitRemoved {
                blobs,
                ..UnitRemoved::default()
            })
        }
        fn lock_blobs(&mut self) -> crate::types::Result<()> {
            self.locked = true;
            Ok(())
        }
        fn referenced(&mut self, candidates: &[String]) -> crate::types::Result<BTreeSet<String>> {
            self.lock_taken_before_check = self.locked;
            self.checks.push(candidates.len());
            Ok(candidates.iter().cloned().collect())
        }
        fn remove_blobs(&mut self, hashes: &[String]) -> crate::types::Result<Vec<(String, u64)>> {
            Ok(hashes.iter().map(|h| (h.clone(), 1)).collect())
        }
    }

    #[test]
    fn every_candidate_is_checked_in_passes_of_a_bounded_size_under_the_blob_lock() {
        let runs: Vec<String> = (0..3).map(|i| format!("run_{i}")).collect();
        let pass = Pass {
            named: runs.iter().map(String::as_str).collect(),
            cutoff_ms: 1,
        };
        let list = anchors(&runs);
        let mut ops = AllReferenced {
            blobs_per_run: REFERENCE_CHUNK,
            checks: Vec::new(),
            lock_taken_before_check: false,
            locked: false,
        };
        let (mut next, mut done, mut report) = (0, BTreeSet::new(), RetentionReport::default());
        process_batch(
            &mut ops,
            &pass,
            &list,
            &mut next,
            usize::MAX,
            &mut done,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.runs_deleted(), 3);
        assert_eq!(report.blobs, 0, "every blob is still referred to");
        assert_eq!(ops.checks.iter().sum::<usize>(), 3 * REFERENCE_CHUNK);
        assert!(ops.checks.len() > 1 && ops.checks.iter().all(|&n| n <= REFERENCE_CHUNK));
        assert!(ops.lock_taken_before_check);
    }

    #[test]
    fn an_apply_batch_stops_after_enough_runs_and_the_next_carries_on() {
        let runs: Vec<String> = (0..BATCH_RUNS + 5).map(|i| format!("run_{i:03}")).collect();
        let pass = Pass {
            named: runs.iter().map(String::as_str).collect(),
            cutoff_ms: 1,
        };
        let list = anchors(&runs);
        let mut ops = AllReferenced {
            blobs_per_run: 1,
            checks: Vec::new(),
            lock_taken_before_check: false,
            locked: false,
        };
        let (mut next, mut done) = (0, BTreeSet::new());
        let mut first = RetentionReport::default();
        process_batch(
            &mut ops, &pass, &list, &mut next, BATCH_RUNS, &mut done, &mut first,
        )
        .unwrap();
        assert_eq!(
            (first.runs_deleted(), next),
            (BATCH_RUNS as u64, BATCH_RUNS)
        );
        let mut second = RetentionReport::default();
        process_batch(
            &mut ops,
            &pass,
            &list,
            &mut next,
            BATCH_RUNS,
            &mut done,
            &mut second,
        )
        .unwrap();
        assert_eq!((second.runs_deleted(), next), (5, BATCH_RUNS + 5));
        assert_eq!(
            ops.checks,
            vec![BATCH_RUNS, 5],
            "one reference check per batch"
        );
    }

    #[test]
    fn an_error_reading_the_texts_is_returned() {
        let texts = vec![Err(crate::types::StoreError::Backend("gone".into()))];
        assert!(named_in_texts(&["a".to_owned()], texts).is_err());
    }
}
