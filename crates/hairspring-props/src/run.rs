//! Aggregation of many trials into per-property run statistics.
//!
//! Two kinds of counts are kept per id. Evidence counts (`trials_hit`, `trials_true`, `trials_false`, `trials_conflicted`)
//! include every trial that catalogued the id, because a failure seen in a crashed or degraded trial is still a
//! failure. Rate counts (`trials_rated` and the interval) use only trials that reported cleanly, so a trial that died
//! before reporting is not mistaken for a trial in which nothing went wrong.

use crate::event::{AssertKind, Loc};
use crate::stats::{rule_of_three, wilson, Z_95};
use crate::trial::{PropertyFailure, TrialProperty, TrialProps, Witness};
use std::collections::{BTreeMap, HashSet};

/// Most kind-mismatch details kept in a [`RunSummary`].
pub const MAX_KIND_MISMATCH_DETAILS: usize = 64;

/// Run-level verdict for one property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyStatus {
    Pass,
    /// A trial-level rule failed in at least one trial (a finding), or the id has conflicting declarations.
    Violation,
    /// A must-hit property was never satisfied in any trial (a coverage gap, not a finding).
    Unmet,
}

/// Which count the reported rate is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateBasis {
    /// Trials with a false evaluation (`always`, `always_or_unreachable`).
    TrialsFalse,
    /// Trials with a true evaluation (`sometimes`).
    TrialsTrue,
    /// Trials that hit the site (`reachable`, `unreachable`).
    TrialsHit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rate {
    pub basis: RateBasis,
    pub k: u64,
    /// Rated trials only (see [`PropertyStat::trials_rated`]).
    pub n: u64,
    pub point: f64,
    /// Wilson 95% interval.
    pub lower: f64,
    pub upper: f64,
    /// `3 / n` when `k == 0`.
    pub rule_of_three: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstFailure {
    /// The lowest seed among the failing trials, whatever order trials were added in.
    pub seed: u64,
    pub witness: Option<Witness>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropertyStat {
    pub id: String,
    pub kind: AssertKind,
    pub must_hit: bool,
    pub msg: String,
    pub loc: Option<Loc>,
    /// Trials in which the id was catalogued.
    pub trials: u64,
    /// Trials that reported cleanly: every process sent `end` and no event data was dropped or contradictory.
    pub trials_rated: u64,
    /// Trials in which a process never sent `end`.
    pub trials_unreported: u64,
    /// Trials with degraded event data.
    pub trials_degraded: u64,
    /// Trials containing `eval` lines; exclude them from tier claims.
    pub trials_perturbing: u64,
    /// Trials in which the id had conflicting declarations.
    pub trials_conflicted: u64,
    pub trials_hit: u64,
    pub trials_true: u64,
    pub trials_false: u64,
    pub evals: u64,
    pub evals_true: u64,
    pub evals_false: u64,
    pub status: PropertyStatus,
    /// Computed over rated trials only.
    pub rate: Option<Rate>,
    pub first_failure: Option<FirstFailure>,
}

/// A trial whose declared kind for an id disagreed with the kind first seen in the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindMismatch {
    pub id: String,
    pub seed: u64,
    pub kept: AssertKind,
    pub seen: AssertKind,
    /// What that trial's own data says, so a violation there is not lost.
    pub failure: Option<PropertyFailure>,
}

/// Counts across the whole run, separate from any one property.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunSummary {
    pub trials: u64,
    /// Trials containing `eval` lines; exclude them from tier claims.
    pub perturbing_trials: u64,
    pub degraded_trials: u64,
    /// Trials in which at least one process sent no `end`.
    pub report_missing_trials: u64,
    /// Trials with a catalog conflict.
    pub conflict_trials: u64,
    /// Trials whose declared kind for an id disagreed with an earlier trial (their data for that id is not merged).
    pub kind_mismatches: u64,
    /// The first [`MAX_KIND_MISMATCH_DETAILS`] mismatches.
    pub kind_mismatch_details: Vec<KindMismatch>,
}

/// How a trial's data may be used for rates.
#[derive(Debug, Clone, Copy)]
struct TrialClass {
    seed: u64,
    unreported: bool,
    degraded: bool,
    perturbing: bool,
}

impl TrialClass {
    fn of(seed: u64, t: &TrialProps) -> Self {
        TrialClass {
            seed,
            unreported: !t.report_missing().is_empty(),
            degraded: t.is_degraded(),
            perturbing: t.perturbing,
        }
    }

    fn is_clean(&self) -> bool {
        !self.unreported && !self.degraded
    }
}

/// Counters for one id across trials.
#[derive(Debug, Clone)]
struct IdAcc {
    kind: AssertKind,
    msg: String,
    loc: Option<Loc>,
    trials: u64,
    rated: u64,
    unreported: u64,
    degraded: u64,
    perturbing: u64,
    conflicted: u64,
    trials_hit: u64,
    trials_true: u64,
    trials_false: u64,
    rated_hit: u64,
    rated_true: u64,
    rated_false: u64,
    evals: u64,
    evals_true: u64,
    evals_false: u64,
    first_failure: Option<FirstFailure>,
}

impl IdAcc {
    fn start(kind: AssertKind, msg: String, loc: Option<Loc>) -> Self {
        IdAcc {
            kind,
            msg,
            loc,
            trials: 0,
            rated: 0,
            unreported: 0,
            degraded: 0,
            perturbing: 0,
            conflicted: 0,
            trials_hit: 0,
            trials_true: 0,
            trials_false: 0,
            rated_hit: 0,
            rated_true: 0,
            rated_false: 0,
            evals: 0,
            evals_true: 0,
            evals_false: 0,
            first_failure: None,
        }
    }

    fn note_failure(&mut self, seed: u64, witness: Option<Witness>) {
        let lower = self.first_failure.as_ref().is_none_or(|f| seed < f.seed);
        if lower {
            self.first_failure = Some(FirstFailure { seed, witness });
        }
    }

    fn with_trial(mut self, class: TrialClass, p: &TrialProperty) -> Self {
        let (hit, tru, fal) = (
            u64::from(p.hits > 0),
            u64::from(p.trues > 0),
            u64::from(p.falses > 0),
        );
        self.trials += 1;
        self.unreported += u64::from(class.unreported);
        self.degraded += u64::from(class.degraded);
        self.perturbing += u64::from(class.perturbing);
        self.trials_hit += hit;
        self.trials_true += tru;
        self.trials_false += fal;
        if class.is_clean() {
            self.rated += 1;
            self.rated_hit += hit;
            self.rated_true += tru;
            self.rated_false += fal;
        }
        self.evals = self.evals.saturating_add(p.hits);
        self.evals_true = self.evals_true.saturating_add(p.trues);
        self.evals_false = self.evals_false.saturating_add(p.falses);
        if let Some(f) = p.failure() {
            self.note_failure(class.seed, f.witness);
        }
        self
    }

    fn with_conflict(mut self, seed: u64) -> Self {
        self.conflicted += 1;
        self.note_failure(seed, None);
        self
    }

    fn status(&self) -> PropertyStatus {
        use AssertKind::*;
        if self.conflicted > 0 {
            return PropertyStatus::Violation;
        }
        match self.kind {
            Always | AlwaysOrUnreachable if self.trials_false > 0 => PropertyStatus::Violation,
            Always if self.trials_hit == 0 => PropertyStatus::Unmet,
            Sometimes if self.trials_true == 0 => PropertyStatus::Unmet,
            Reachable if self.trials_hit == 0 => PropertyStatus::Unmet,
            Unreachable if self.trials_hit > 0 => PropertyStatus::Violation,
            _ => PropertyStatus::Pass,
        }
    }

    fn rate(&self) -> Option<Rate> {
        let (basis, k) = match self.kind {
            AssertKind::Always | AssertKind::AlwaysOrUnreachable => {
                (RateBasis::TrialsFalse, self.rated_false)
            }
            AssertKind::Sometimes => (RateBasis::TrialsTrue, self.rated_true),
            AssertKind::Reachable | AssertKind::Unreachable => {
                (RateBasis::TrialsHit, self.rated_hit)
            }
        };
        let n = self.rated;
        let (lower, upper) = wilson(k, n, Z_95)?;
        Some(Rate {
            basis,
            k,
            n,
            point: k as f64 / n as f64,
            lower,
            upper,
            rule_of_three: if k == 0 { rule_of_three(n) } else { None },
        })
    }

    fn into_stat(self, id: String) -> PropertyStat {
        let status = self.status();
        let rate = self.rate();
        PropertyStat {
            id,
            kind: self.kind,
            must_hit: self.kind.must_hit(),
            msg: self.msg,
            loc: self.loc,
            trials: self.trials,
            trials_rated: self.rated,
            trials_unreported: self.unreported,
            trials_degraded: self.degraded,
            trials_perturbing: self.perturbing,
            trials_conflicted: self.conflicted,
            trials_hit: self.trials_hit,
            trials_true: self.trials_true,
            trials_false: self.trials_false,
            evals: self.evals,
            evals_true: self.evals_true,
            evals_false: self.evals_false,
            status,
            rate,
            first_failure: self.first_failure,
        }
    }
}

/// Accumulator over trials. Consumed and returned by value, so no accumulator is ever shared or mutated in place.
#[derive(Debug, Clone, Default)]
pub struct RunProps {
    summary: RunSummary,
    ids: BTreeMap<String, IdAcc>,
}

impl RunProps {
    pub fn new() -> Self {
        RunProps::default()
    }

    /// Fold one trial in. `seed` identifies the trial in first-failure reports.
    #[must_use]
    pub fn add(self, seed: u64, trial: &TrialProps) -> RunProps {
        let RunProps {
            mut summary,
            mut ids,
        } = self;
        let class = TrialClass::of(seed, trial);
        summary.trials += 1;
        summary.perturbing_trials += u64::from(class.perturbing);
        summary.degraded_trials += u64::from(class.degraded);
        summary.report_missing_trials += u64::from(class.unreported);
        summary.conflict_trials += u64::from(!trial.conflicts.is_empty());
        for p in &trial.properties {
            let acc = ids
                .remove(&p.id)
                .unwrap_or_else(|| IdAcc::start(p.kind, p.msg.clone(), p.loc.clone()));
            let acc = if acc.kind == p.kind {
                acc.with_trial(class, p)
            } else {
                summary.kind_mismatches += 1;
                if summary.kind_mismatch_details.len() < MAX_KIND_MISMATCH_DETAILS {
                    summary.kind_mismatch_details.push(KindMismatch {
                        id: p.id.clone(),
                        seed,
                        kept: acc.kind,
                        seen: p.kind,
                        failure: p.failure(),
                    });
                }
                acc
            };
            ids.insert(p.id.clone(), acc);
        }
        let conflicted: HashSet<&str> = trial.conflicts.iter().map(|c| c.id.as_str()).collect();
        for id in conflicted {
            if let Some(acc) = ids.remove(id) {
                ids.insert(id.to_owned(), acc.with_conflict(seed));
            }
        }
        RunProps { summary, ids }
    }

    pub fn summary(&self) -> RunSummary {
        self.summary.clone()
    }

    /// Per-id statistics sorted by id.
    pub fn finish(self) -> Vec<PropertyStat> {
        self.ids
            .into_iter()
            .map(|(id, acc)| acc.into_stat(id))
            .collect()
    }
}
