//! The run fold exactly as it was before trials were folded through digests (copied verbatim from the previous
//! `run.rs`, with the types it builds imported from the library). It is the oracle for the equivalence tests: the
//! digest fold must produce byte-identical statistics for every input.
#![allow(dead_code, clippy::all)]

use hairspring_props::limits::MAX_IDS_PER_RUN;
use hairspring_props::run::MAX_KIND_MISMATCH_DETAILS;
use hairspring_props::stats::{rule_of_three, wilson, Z_95};
use hairspring_props::{
    AssertKind, FirstFailure, KindMismatch, Loc, PropertyStat, PropertyStatus, Rate, RateBasis,
    RunSummary, TrialProperty, TrialProps, UnratedReason, Witness,
};
use std::collections::{BTreeMap, HashSet};

/// How a trial's data may be used for rates.
#[derive(Debug, Clone, Copy)]
struct TrialClass {
    seed: u64,
    unreported: bool,
    degraded: bool,
    perturbing: bool,
    /// The trial was inconclusive (it timed out): its data is evidence of nothing.
    inconclusive: bool,
}

impl TrialClass {
    fn of(seed: u64, t: &TrialProps, inconclusive: bool) -> Self {
        TrialClass {
            seed,
            unreported: inconclusive || !t.report_missing().is_empty(),
            degraded: t.is_degraded(),
            perturbing: t.perturbing,
            inconclusive,
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
    /// False while only inconclusive trials have been seen: their data decides neither the kind nor the message.
    decided: bool,
    trials: u64,
    rated: u64,
    unreported: u64,
    degraded: u64,
    perturbing: u64,
    conflicted: u64,
    inconclusive_failures: u64,
    trials_hit: u64,
    trials_true: u64,
    trials_false: u64,
    rated_hit: u64,
    rated_true: u64,
    rated_false: u64,
    claimable: u64,
    claim_hit: u64,
    claim_true: u64,
    evals: u64,
    evals_true: u64,
    evals_false: u64,
    first_failure: Option<FirstFailure>,
}

impl IdAcc {
    fn start(p: &TrialProperty, decided: bool) -> Self {
        IdAcc {
            kind: p.kind,
            msg: p.msg.clone(),
            loc: p.loc.clone(),
            decided,
            trials: 0,
            rated: 0,
            unreported: 0,
            degraded: 0,
            perturbing: 0,
            conflicted: 0,
            inconclusive_failures: 0,
            trials_hit: 0,
            trials_true: 0,
            trials_false: 0,
            rated_hit: 0,
            rated_true: 0,
            rated_false: 0,
            claimable: 0,
            claim_hit: 0,
            claim_true: 0,
            evals: 0,
            evals_true: 0,
            evals_false: 0,
            first_failure: None,
        }
    }

    /// The first conclusive trial settles the kind, message and location. A failure counted from the inconclusive
    /// trials before it was seen under a kind that was never decided, so it is forgotten when the kind changes.
    fn decide(self, p: &TrialProperty) -> Self {
        let kept = self.kind == p.kind;
        IdAcc {
            kind: p.kind,
            msg: p.msg.clone(),
            loc: p.loc.clone(),
            decided: true,
            inconclusive_failures: if kept { self.inconclusive_failures } else { 0 },
            ..self
        }
    }

    fn note_failure(&mut self, seed: u64, witness: Option<Witness>) {
        let lower = self.first_failure.as_ref().is_none_or(|f| seed < f.seed);
        if lower {
            self.first_failure = Some(FirstFailure { seed, witness });
        }
    }

    fn with_trial(mut self, class: TrialClass, p: &TrialProperty) -> Self {
        if class.inconclusive {
            self.trials += 1;
            self.unreported += 1;
            self.perturbing += u64::from(class.perturbing);
            self.inconclusive_failures += u64::from(p.failure().is_some());
            return self;
        }
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
            if !class.perturbing {
                self.claimable += 1;
                self.claim_hit += hit;
                self.claim_true += tru;
            }
        }
        self.evals = self.evals.saturating_add(p.hits);
        self.evals_true = self.evals_true.saturating_add(p.trues);
        self.evals_false = self.evals_false.saturating_add(p.falses);
        if let Some(f) = p.failure() {
            self.note_failure(class.seed, f.witness);
        }
        self
    }

    /// Counts a timed-out trial whose kind differs from this id's: an unreported trial, nothing else.
    fn with_unjudged_trial(mut self, class: TrialClass) -> Self {
        self.trials += 1;
        self.unreported += 1;
        self.perturbing += u64::from(class.perturbing);
        self
    }

    fn with_conflict(mut self, seed: u64) -> Self {
        self.conflicted += 1;
        self.note_failure(seed, None);
        self
    }

    /// Failures count whatever trial they came from (a false is a false). Must-hit claims, and the passes that
    /// depend on them, are made over claimable trials only, so a degraded or perturbing trial can neither supply the
    /// hit that satisfies a property nor stand in for a clean trial.
    fn status(&self) -> PropertyStatus {
        use AssertKind::*;
        if self.conflicted > 0 {
            return PropertyStatus::Violation;
        }
        match self.kind {
            Always | AlwaysOrUnreachable if self.trials_false > 0 => {
                return PropertyStatus::Violation
            }
            Unreachable if self.trials_hit > 0 => return PropertyStatus::Violation,
            _ => {}
        }
        if self.claimable == 0 {
            return PropertyStatus::Unrated;
        }
        match self.kind {
            Always | Reachable if self.claim_hit == 0 => PropertyStatus::Unmet,
            Sometimes if self.claim_true == 0 => PropertyStatus::Unmet,
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
            trials_inconclusive_with_failures: self.inconclusive_failures,
            trials_claimable: self.claimable,
            trials_hit: self.trials_hit,
            trials_true: self.trials_true,
            trials_false: self.trials_false,
            evals: self.evals,
            evals_true: self.evals_true,
            evals_false: self.evals_false,
            status,
            unrated_reason: rate.is_none().then_some(UnratedReason::NoRatedTrials),
            rate,
            first_failure: self.first_failure,
        }
    }
}

/// Accumulator over trials. Consumed and returned by value, so no accumulator is ever shared or mutated in place.
#[derive(Debug, Clone)]
pub struct LegacyRunProps {
    summary: RunSummary,
    ids: BTreeMap<String, IdAcc>,
    id_cap: usize,
}

impl Default for LegacyRunProps {
    fn default() -> Self {
        LegacyRunProps::with_id_cap(MAX_IDS_PER_RUN)
    }
}

impl LegacyRunProps {
    /// An accumulator tracking at most [`MAX_IDS_PER_RUN`] distinct ids.
    pub fn new() -> Self {
        LegacyRunProps::default()
    }

    /// An accumulator tracking at most `id_cap` distinct ids. Ids past the cap are counted in
    /// [`RunSummary::ids_dropped`] and the trial that lost them is counted in `degraded_trials`; no per-id statistic
    /// changes. Timed-out trials take cap slots like any other, so ids first seen only in timed-out trials can crowd
    /// out ids that later trials report.
    pub fn with_id_cap(id_cap: usize) -> Self {
        LegacyRunProps {
            summary: RunSummary {
                id_cap,
                ..RunSummary::default()
            },
            ids: BTreeMap::new(),
            id_cap,
        }
    }

    /// Fold one trial in. `seed` identifies the trial in first-failure reports.
    #[must_use]
    pub fn add(self, seed: u64, trial: &TrialProps) -> LegacyRunProps {
        self.add_classified(seed, trial, false)
    }

    /// Fold in a trial that was inconclusive (it timed out). Its report cannot be trusted, so it is counted as
    /// unreported, adds no hit, true or false counts and no first failure, and cannot make a property a violation;
    /// a failure it showed is only counted in `trials_inconclusive_with_failures`.
    #[must_use]
    pub fn add_inconclusive(self, seed: u64, trial: &TrialProps) -> LegacyRunProps {
        self.add_classified(seed, trial, true)
    }

    fn add_classified(self, seed: u64, trial: &TrialProps, inconclusive: bool) -> LegacyRunProps {
        let LegacyRunProps {
            mut summary,
            mut ids,
            id_cap,
        } = self;
        let class = TrialClass::of(seed, trial, inconclusive);
        let mut dropped = 0u64;
        for p in &trial.properties {
            let acc = match ids.remove(&p.id) {
                Some(acc) => acc,
                None if ids.len() >= id_cap => {
                    dropped += 1;
                    continue;
                }
                None => IdAcc::start(p, !inconclusive),
            };
            let acc = match (acc.decided, inconclusive) {
                (false, false) => acc.decide(p),
                _ => acc,
            };
            let acc = if acc.kind == p.kind {
                acc.with_trial(class, p)
            } else if inconclusive {
                // A timed-out trial's kind is not trusted: it decides nothing and is no mismatch. It still counts as
                // an unreported trial of the id, but its failure is not attributed to a kind it may not have.
                acc.with_unjudged_trial(class)
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
        let conflicted: HashSet<&str> = trial
            .conflicts
            .iter()
            .filter(|_| !inconclusive)
            .map(|c| c.id.as_str())
            .collect();
        for id in conflicted {
            if let Some(acc) = ids.remove(id) {
                ids.insert(id.to_owned(), acc.with_conflict(seed));
            }
        }
        let lost_ids = dropped > 0;
        summary.trials += 1;
        summary.perturbing_trials += u64::from(class.perturbing);
        summary.degraded_trials += u64::from(class.degraded || lost_ids);
        summary.inconclusive_trials += u64::from(inconclusive);
        summary.report_missing_trials += u64::from(class.unreported && !inconclusive);
        summary.conflict_trials += u64::from(!inconclusive && !trial.conflicts.is_empty());
        summary.ids_dropped = summary.ids_dropped.saturating_add(dropped);
        LegacyRunProps {
            summary,
            ids,
            id_cap,
        }
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
