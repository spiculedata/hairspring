//! Aggregation of many trials into per-property run statistics.

use crate::event::{AssertKind, Loc};
use crate::stats::{rule_of_three, wilson, Z_95};
use crate::trial::{TrialProperty, TrialProps, Witness};
use std::collections::BTreeMap;

/// Run-level verdict for one property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyStatus {
    Pass,
    /// A trial-level rule failed in at least one trial (a finding).
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
    pub trials_hit: u64,
    pub trials_true: u64,
    pub trials_false: u64,
    pub evals: u64,
    pub evals_true: u64,
    pub evals_false: u64,
    pub status: PropertyStatus,
    pub rate: Option<Rate>,
    pub first_failure: Option<FirstFailure>,
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
    /// Trials with a within-trial catalog conflict.
    pub conflict_trials: u64,
    /// Trials whose declared kind for an id disagreed with an earlier trial (their data for that id is ignored).
    pub kind_mismatches: u64,
}

/// Counters for one id across trials.
#[derive(Debug, Clone)]
struct IdAcc {
    kind: AssertKind,
    msg: String,
    loc: Option<Loc>,
    trials: u64,
    trials_hit: u64,
    trials_true: u64,
    trials_false: u64,
    evals: u64,
    evals_true: u64,
    evals_false: u64,
    first_failure: Option<FirstFailure>,
}

impl IdAcc {
    fn start(p: &TrialProperty) -> Self {
        IdAcc {
            kind: p.kind,
            msg: p.msg.clone(),
            loc: p.loc.clone(),
            trials: 0,
            trials_hit: 0,
            trials_true: 0,
            trials_false: 0,
            evals: 0,
            evals_true: 0,
            evals_false: 0,
            first_failure: None,
        }
    }

    fn with_trial(mut self, seed: u64, p: &TrialProperty) -> Self {
        self.trials += 1;
        self.trials_hit += u64::from(p.hits > 0);
        self.trials_true += u64::from(p.trues > 0);
        self.trials_false += u64::from(p.falses > 0);
        self.evals = self.evals.saturating_add(p.hits);
        self.evals_true = self.evals_true.saturating_add(p.trues);
        self.evals_false = self.evals_false.saturating_add(p.falses);
        if self.first_failure.is_none() {
            self.first_failure = p.failure().map(|f| FirstFailure {
                seed,
                witness: f.witness,
            });
        }
        self
    }

    fn status(&self) -> PropertyStatus {
        use AssertKind::*;
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
                (RateBasis::TrialsFalse, self.trials_false)
            }
            AssertKind::Sometimes => (RateBasis::TrialsTrue, self.trials_true),
            AssertKind::Reachable | AssertKind::Unreachable => {
                (RateBasis::TrialsHit, self.trials_hit)
            }
        };
        let n = self.trials;
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
        summary.trials += 1;
        summary.perturbing_trials += u64::from(trial.perturbing);
        summary.degraded_trials += u64::from(trial.is_degraded());
        summary.report_missing_trials += u64::from(!trial.report_missing().is_empty());
        summary.conflict_trials += u64::from(!trial.conflicts.is_empty());
        for p in &trial.properties {
            let acc = ids.remove(&p.id).unwrap_or_else(|| IdAcc::start(p));
            let acc = if acc.kind == p.kind {
                acc.with_trial(seed, p)
            } else {
                summary.kind_mismatches += 1;
                acc
            };
            ids.insert(p.id.clone(), acc);
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
