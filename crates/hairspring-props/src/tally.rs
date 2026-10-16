//! The cheap tier of a subtree fold: verdict counts plus a capped set of failing catalog ids.
//!
//! A [`NodeTally`] is built from a verdict-count query, or from stored trials one at a time, and combined with
//! [`NodeTally::merge`], which is associative and commutative, so a subtree can be folded in any grouping. It carries
//! counts and ids only. It has no rate and no interval: the trials under a tree node were chosen by exploration, not
//! drawn at random, so a share of them says nothing about how often a property fails.
//!
//! Verdicts come from the store's verdict column, never from a digest: a trial can be a violation with no failing
//! property (an exit code, a hang), and a sim or WASM trial's digest has no property entries at all.

use crate::catalog::RunCatalog;
use crate::digest::TrialDigest;
use crate::trial::failure_basis;
use std::collections::BTreeSet;

/// Most failing catalog ids one tally keeps. Past it the tally keeps the lowest ids and says it is truncated.
pub const MAX_TALLY_FAILING_IDS: usize = 256;

/// Trial counts by the verdict names the store uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VerdictCounts {
    pub pass: u64,
    pub violation: u64,
    pub inconclusive: u64,
    pub unusable: u64,
    pub infra: u64,
    /// Trials whose verdict name this build does not know. Never merged into another count.
    pub unknown: u64,
}

impl VerdictCounts {
    /// These counts with `n` more trials of the verdict called `name`.
    #[must_use]
    pub fn with_named(self, name: &str, n: u64) -> VerdictCounts {
        let add = |c: u64| c.saturating_add(n);
        match name {
            "pass" => VerdictCounts {
                pass: add(self.pass),
                ..self
            },
            "violation" => VerdictCounts {
                violation: add(self.violation),
                ..self
            },
            "inconclusive" => VerdictCounts {
                inconclusive: add(self.inconclusive),
                ..self
            },
            "unusable" => VerdictCounts {
                unusable: add(self.unusable),
                ..self
            },
            "infra" => VerdictCounts {
                infra: add(self.infra),
                ..self
            },
            _ => VerdictCounts {
                unknown: add(self.unknown),
                ..self
            },
        }
    }

    /// Every counted trial. Saturates at `u64::MAX`.
    pub fn total(&self) -> u64 {
        [
            self.pass,
            self.violation,
            self.inconclusive,
            self.unusable,
            self.infra,
            self.unknown,
        ]
        .into_iter()
        .fold(0, u64::saturating_add)
    }

    fn merge(&self, o: &VerdictCounts) -> VerdictCounts {
        VerdictCounts {
            pass: self.pass.saturating_add(o.pass),
            violation: self.violation.saturating_add(o.violation),
            inconclusive: self.inconclusive.saturating_add(o.inconclusive),
            unusable: self.unusable.saturating_add(o.unusable),
            infra: self.infra.saturating_add(o.infra),
            unknown: self.unknown.saturating_add(o.unknown),
        }
    }
}

/// Verdict counts and failing ids for a set of trials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeTally {
    verdicts: VerdictCounts,
    excluded: u64,
    degraded: u64,
    perturbing: u64,
    failing_ids: BTreeSet<u32>,
    /// Ids were cut to [`MAX_TALLY_FAILING_IDS`].
    failing_ids_truncated: bool,
    /// Every trial in the tally either told which of its properties failed or has none to tell. False once a trial
    /// of unknown makeup is merged in (a count query, a violation with no digest, a conflict-only entry).
    failing_ids_complete: bool,
}

impl Default for NodeTally {
    fn default() -> Self {
        NodeTally::empty()
    }
}

impl NodeTally {
    /// A tally of nothing.
    pub fn empty() -> NodeTally {
        NodeTally {
            verdicts: VerdictCounts::default(),
            excluded: 0,
            degraded: 0,
            perturbing: 0,
            failing_ids: BTreeSet::new(),
            failing_ids_truncated: false,
            failing_ids_complete: true,
        }
    }

    /// A tally from verdict counts alone (the result of a grouped count query). It names no failing ids, so when it
    /// holds violations its id set is not exhaustive.
    pub fn from_verdicts(verdicts: VerdictCounts) -> NodeTally {
        NodeTally {
            verdicts,
            failing_ids_complete: verdicts.violation == 0,
            ..NodeTally::empty()
        }
    }

    /// The tally of one stored trial. `verdict` is the store's verdict name and alone decides the verdict count, so
    /// the counts equal [`NodeTally::from_verdicts`] over the same rows. `digest`, when there is one, adds only
    /// - the failing ids, judged against `catalog` as the run fold judges them: an entry counts under the catalog's
    ///   kind (a kind mismatch is not merged), and a conflict on an entry that reported counts is a failure;
    /// - the excluded, degraded and perturbing counts, by the run fold's rules (an excluded trial adds only to
    ///   `excluded`; a timed-out trial names no failing ids but still counts as degraded or perturbing).
    ///
    /// A conflict-only entry may fail an id this trial alone cannot see, and a violation with no digest has unknown
    /// ids; either leaves the id set not exhaustive.
    pub fn of_trial(
        verdict: &str,
        digest: Option<&TrialDigest>,
        catalog: &RunCatalog,
    ) -> NodeTally {
        let base = NodeTally {
            verdicts: VerdictCounts::default().with_named(verdict, 1),
            ..NodeTally::empty()
        };
        let Some(d) = digest else {
            return NodeTally {
                failing_ids_complete: verdict != "violation",
                ..base
            };
        };
        if d.flags.excluded {
            return NodeTally {
                excluded: 1,
                ..base
            };
        }
        let counted = NodeTally {
            degraded: u64::from(d.flags.degraded || d.ids_dropped > 0),
            perturbing: u64::from(d.flags.perturbing),
            ..base
        };
        if d.flags.inconclusive {
            return counted;
        }
        let failing: BTreeSet<u32> = d
            .entries
            .iter()
            .filter(|e| entry_fails(e, catalog))
            .map(|e| e.idx)
            .collect();
        let unseen = d
            .entries
            .iter()
            .any(|e| e.conflicted && !e.catalogued && catalog.get(e.idx).is_some());
        NodeTally {
            failing_ids_complete: !unseen,
            ..counted
        }
        .with_failing(failing, false)
    }

    /// This tally with its failing ids replaced by `ids`, which the caller knows to be every failing id (the run fold
    /// has seen the whole subtree). Cut to the cap and flagged as usual.
    pub(crate) fn with_known_failing_ids(self, ids: BTreeSet<u32>) -> NodeTally {
        NodeTally {
            failing_ids: BTreeSet::new(),
            failing_ids_truncated: false,
            failing_ids_complete: true,
            ..self
        }
        .with_failing(ids, false)
    }

    /// The two tallies combined. Associative and commutative, whatever the inputs: counts add (saturating at
    /// `u64::MAX`), failing ids are united and cut to the lowest [`MAX_TALLY_FAILING_IDS`], the result is flagged
    /// truncated when either side was or the union was cut, and it is exhaustive only when both sides were.
    #[must_use]
    pub fn merge(&self, other: &NodeTally) -> NodeTally {
        NodeTally {
            verdicts: self.verdicts.merge(&other.verdicts),
            excluded: self.excluded.saturating_add(other.excluded),
            degraded: self.degraded.saturating_add(other.degraded),
            perturbing: self.perturbing.saturating_add(other.perturbing),
            failing_ids: BTreeSet::new(),
            failing_ids_truncated: self.failing_ids_truncated || other.failing_ids_truncated,
            failing_ids_complete: self.failing_ids_complete && other.failing_ids_complete,
        }
        .with_failing(
            self.failing_ids
                .union(&other.failing_ids)
                .copied()
                .collect(),
            false,
        )
    }

    fn with_failing(self, ids: BTreeSet<u32>, was_truncated: bool) -> NodeTally {
        let cut = ids.len() > MAX_TALLY_FAILING_IDS;
        NodeTally {
            failing_ids: ids.into_iter().take(MAX_TALLY_FAILING_IDS).collect(),
            failing_ids_truncated: self.failing_ids_truncated || was_truncated || cut,
            ..self
        }
    }

    pub fn verdicts(&self) -> &VerdictCounts {
        &self.verdicts
    }

    /// Trials whose digest was excluded from all statistics because its data could not be trusted. These trials are
    /// also in [`NodeTally::verdicts`] under the store's verdict.
    pub fn excluded(&self) -> u64 {
        self.excluded
    }

    /// Trials with degraded event data, or that lost ids to a cap, counted as `RunSummary::degraded_trials` counts
    /// them: a timed-out trial counts, an excluded one does not. This is the run-level trial count; the
    /// per-property `trials_degraded` of a stat leaves timed-out trials out, so the two can differ.
    pub fn degraded(&self) -> u64 {
        self.degraded
    }

    /// Trials containing per-evaluation lines.
    pub fn perturbing(&self) -> u64 {
        self.perturbing
    }

    /// Catalog indexes of ids that failed in a conclusive trial, lowest first, at most [`MAX_TALLY_FAILING_IDS`].
    pub fn failing_ids(&self) -> &BTreeSet<u32> {
        &self.failing_ids
    }

    /// True when more ids failed than [`NodeTally::failing_ids`] holds. The verdict counts are never truncated.
    pub fn failing_ids_truncated(&self) -> bool {
        self.failing_ids_truncated
    }

    /// True when every trial in the tally told which properties failed, so [`NodeTally::failing_ids`] is the whole
    /// set, apart from a cut shown by [`NodeTally::failing_ids_truncated`].
    pub fn failing_ids_complete(&self) -> bool {
        self.failing_ids_complete
    }

    /// True when `failing_ids` is every failing id: complete and not cut. Show "and others" or "unknown" when false.
    pub fn failing_ids_exhaustive(&self) -> bool {
        self.failing_ids_complete && !self.failing_ids_truncated
    }
}

/// Whether one entry fails its id the way the run fold would count it, given the catalog's kind for the id.
fn entry_fails(e: &crate::digest::DigestEntry, catalog: &RunCatalog) -> bool {
    let Some(row) = catalog.get(e.idx) else {
        return false;
    };
    let counted_failure =
        e.catalogued && row.kind == e.kind && failure_basis(e.kind, e.hits, e.falses).is_some();
    counted_failure || (e.catalogued && e.conflicted)
}
