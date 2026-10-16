//! The cheap tier of a subtree fold: verdict counts plus a capped set of failing catalog ids.
//!
//! A [`NodeTally`] is built from a verdict-count query, or from digests one at a time, and combined with
//! [`NodeTally::merge`], which is associative and commutative, so a subtree can be folded in any grouping. It carries
//! counts and ids only. It has no rate and no interval: the trials under a tree node were chosen by exploration, not
//! drawn at random, so a share of them says nothing about how often a property fails.

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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeTally {
    verdicts: VerdictCounts,
    excluded: u64,
    degraded: u64,
    perturbing: u64,
    failing_ids: BTreeSet<u32>,
    failing_ids_truncated: bool,
}

impl NodeTally {
    /// A tally of nothing.
    pub fn empty() -> NodeTally {
        NodeTally::default()
    }

    /// A tally from verdict counts alone (the result of a grouped count query). It names no failing ids.
    pub fn from_verdicts(verdicts: VerdictCounts) -> NodeTally {
        NodeTally {
            verdicts,
            ..NodeTally::default()
        }
    }

    /// The tally of one trial. An excluded trial adds only to [`NodeTally::excluded`]; an inconclusive one is
    /// counted as inconclusive and names no failing ids, as the run fold treats it.
    pub fn of_digest(digest: &TrialDigest) -> NodeTally {
        if digest.flags.excluded {
            return NodeTally {
                excluded: 1,
                ..NodeTally::default()
            };
        }
        let perturbing = u64::from(digest.flags.perturbing);
        if digest.flags.inconclusive {
            return NodeTally {
                verdicts: VerdictCounts::default().with_named("inconclusive", 1),
                perturbing,
                ..NodeTally::default()
            };
        }
        let failing: BTreeSet<u32> = digest
            .entries
            .iter()
            .filter(|e| {
                (e.catalogued && failure_basis(e.kind, e.hits, e.falses).is_some()) || e.conflicted
            })
            .map(|e| e.idx)
            .collect();
        let verdict = if failing.is_empty() {
            "pass"
        } else {
            "violation"
        };
        NodeTally {
            verdicts: VerdictCounts::default().with_named(verdict, 1),
            excluded: 0,
            degraded: u64::from(digest.flags.degraded || digest.ids_dropped > 0),
            perturbing,
            ..NodeTally::default()
        }
        .with_failing(failing, false)
    }

    /// The two tallies combined. Associative and commutative, whatever the inputs: counts add (saturating at
    /// `u64::MAX`), failing ids are united and cut to the lowest [`MAX_TALLY_FAILING_IDS`], and the result is
    /// flagged truncated when either side was or the union was cut.
    #[must_use]
    pub fn merge(&self, other: &NodeTally) -> NodeTally {
        NodeTally {
            verdicts: self.verdicts.merge(&other.verdicts),
            excluded: self.excluded.saturating_add(other.excluded),
            degraded: self.degraded.saturating_add(other.degraded),
            perturbing: self.perturbing.saturating_add(other.perturbing),
            failing_ids: BTreeSet::new(),
            failing_ids_truncated: false,
        }
        .with_failing(
            self.failing_ids
                .union(&other.failing_ids)
                .copied()
                .collect(),
            self.failing_ids_truncated || other.failing_ids_truncated,
        )
    }

    fn with_failing(self, ids: BTreeSet<u32>, was_truncated: bool) -> NodeTally {
        let cut = ids.len() > MAX_TALLY_FAILING_IDS;
        NodeTally {
            failing_ids: ids.into_iter().take(MAX_TALLY_FAILING_IDS).collect(),
            failing_ids_truncated: was_truncated || cut,
            ..self
        }
    }

    pub fn verdicts(&self) -> &VerdictCounts {
        &self.verdicts
    }

    /// Trials left out of all statistics because their data could not be trusted.
    pub fn excluded(&self) -> u64 {
        self.excluded
    }

    /// Conclusive trials with degraded event data, or that lost ids to the run cap.
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
}
