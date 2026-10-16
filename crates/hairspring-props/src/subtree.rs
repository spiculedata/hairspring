//! Folding the digests under one tree node.
//!
//! The statistics come from [`RunProps`], so a subtree fold agrees with a run fold over the same digests by
//! construction. What differs is what is reported. The trials under a tree node were reached by exploration, not drawn
//! at random, so a share of them is not how often a property fails: [`SubtreeStat`] carries counts and a violation
//! flag, and no rate, interval, pass or unmet status.

use crate::catalog::RunCatalog;
use crate::digest::TrialDigest;
use crate::event::{AssertKind, Loc};
use crate::run::{FirstFailure, PropertyStat, PropertyStatus, RunProps, RunSummary};
use crate::tally::NodeTally;
use std::borrow::Borrow;
use std::fmt;

/// Most digests one subtree fold reads. A larger subtree is folded child by child.
pub const MAX_DIGESTS_PER_FOLD: usize = 100_000;

/// Why a subtree could not be folded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldError {
    /// The stream held more than `limit` digests.
    LimitExceeded { limit: usize },
}

impl fmt::Display for FoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FoldError::LimitExceeded { limit } => {
                write!(
                    f,
                    "subtree has more than {limit} trials: fold the children instead"
                )
            }
        }
    }
}

impl std::error::Error for FoldError {}

/// One property's counts over a subtree. The counters mean what they mean in [`PropertyStat`].
#[derive(Debug, Clone, PartialEq)]
pub struct SubtreeStat {
    pub id: String,
    pub kind: AssertKind,
    pub must_hit: bool,
    pub msg: String,
    pub loc: Option<Loc>,
    pub trials: u64,
    pub trials_rated: u64,
    pub trials_unreported: u64,
    pub trials_degraded: u64,
    pub trials_perturbing: u64,
    pub trials_conflicted: u64,
    pub trials_inconclusive_with_failures: u64,
    pub trials_claimable: u64,
    pub trials_hit: u64,
    pub trials_true: u64,
    pub trials_false: u64,
    pub evals: u64,
    pub evals_true: u64,
    pub evals_false: u64,
    /// A conclusive trial in the subtree broke the property (or the id had conflicting declarations).
    pub violated: bool,
    pub first_failure: Option<FirstFailure>,
}

impl From<PropertyStat> for SubtreeStat {
    fn from(s: PropertyStat) -> SubtreeStat {
        SubtreeStat {
            violated: s.status == PropertyStatus::Violation,
            id: s.id,
            kind: s.kind,
            must_hit: s.must_hit,
            msg: s.msg,
            loc: s.loc,
            trials: s.trials,
            trials_rated: s.trials_rated,
            trials_unreported: s.trials_unreported,
            trials_degraded: s.trials_degraded,
            trials_perturbing: s.trials_perturbing,
            trials_conflicted: s.trials_conflicted,
            trials_inconclusive_with_failures: s.trials_inconclusive_with_failures,
            trials_claimable: s.trials_claimable,
            trials_hit: s.trials_hit,
            trials_true: s.trials_true,
            trials_false: s.trials_false,
            evals: s.evals,
            evals_true: s.evals_true,
            evals_false: s.evals_false,
            first_failure: s.first_failure,
        }
    }
}

/// The full fold of one subtree.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtreeFold {
    pub tally: NodeTally,
    /// The run fold's summary over these digests. Its `kind_mismatch_details` hold text a guest process wrote (ids,
    /// messages, locations): a reader must fence it as untrusted before showing it. Its `degraded_trials` counts
    /// trials, timed-out ones included, and can differ from the per-property `trials_degraded` of the stats.
    pub summary: RunSummary,
    /// Per-property counts sorted by id.
    pub stats: Vec<SubtreeStat>,
}

/// Folds a stream of `(seed, verdict, digest)` triples whose indexes refer to `catalog` (a tree catalog, after
/// [`TrialDigest::remap`]). `seed` identifies a trial in first-failure reports. `verdict` is the store's verdict
/// name for the trial; it alone decides the tally's verdict counts, because a digest cannot show a violation that
/// has no failing property.
///
/// The tally's failing ids are the properties the run fold reports as violated, so they match [`RunProps`] exactly,
/// including its handling of kind mismatches and conflict-only entries.
///
/// Reads at most [`MAX_DIGESTS_PER_FOLD`] digests; one more is an error, and the stream is not read further.
pub fn fold_subtree<I, V, D>(trials: I, catalog: &RunCatalog) -> Result<SubtreeFold, FoldError>
where
    I: IntoIterator<Item = (u64, V, D)>,
    V: AsRef<str>,
    D: Borrow<TrialDigest>,
{
    let mut read = 0usize;
    let mut tally = NodeTally::empty();
    let mut run = RunProps::new();
    for (seed, verdict, digest) in trials {
        read += 1;
        if read > MAX_DIGESTS_PER_FOLD {
            return Err(FoldError::LimitExceeded {
                limit: MAX_DIGESTS_PER_FOLD,
            });
        }
        let digest = digest.borrow();
        tally = tally.merge(&NodeTally::of_trial(
            verdict.as_ref(),
            Some(digest),
            catalog,
        ));
        run = run.add_digest(seed, digest, catalog);
    }
    let summary = run.summary();
    let stats: Vec<SubtreeStat> = run.finish().into_iter().map(SubtreeStat::from).collect();
    let violated = stats
        .iter()
        .filter(|s| s.violated)
        .filter_map(|s| catalog.index_of(&s.id))
        .collect();
    Ok(SubtreeFold {
        tally: tally.with_known_failing_ids(violated),
        summary,
        stats,
    })
}
