//! Aggregation of many trials into per-property run statistics.
//!
//! Two kinds of counts are kept per id. Evidence counts (`trials_hit`, `trials_true`, `trials_false`, `trials_conflicted`)
//! include every trial that catalogued the id, because a failure seen in a crashed or degraded trial is still a
//! failure. Rate counts (`trials_rated` and the interval) use only trials that reported cleanly, so a trial that died
//! before reporting is not mistaken for a trial in which nothing went wrong.

use crate::catalog::RunCatalog;
use crate::digest::{DigestEntry, TrialDigest};
use crate::event::{AssertKind, Loc};
use crate::limits::MAX_IDS_PER_RUN;
use crate::stats::{rule_of_three, wilson, Z_95};
use crate::trial::{failure_basis, FailureReason, PropertyFailure, TrialProps, Witness};
use std::collections::BTreeMap;

/// Most kind-mismatch details kept in a [`RunSummary`].
pub const MAX_KIND_MISMATCH_DETAILS: usize = 64;

/// Run-level verdict for one property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PropertyStatus {
    Pass,
    /// A trial-level rule failed in at least one trial (a finding), or the id has conflicting declarations.
    Violation,
    /// A must-hit property was never satisfied in any claimable trial (a coverage gap, not a finding).
    Unmet,
    /// No trial reported cleanly and unperturbed, so neither a pass nor an UNMET can be claimed. A failure seen in
    /// an unclean trial is still a [`PropertyStatus::Violation`].
    Unrated,
}

/// Which count the reported rate is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RateBasis {
    /// Trials with a false evaluation (`always`, `always_or_unreachable`).
    TrialsFalse,
    /// Trials with a true evaluation (`sometimes`).
    TrialsTrue,
    /// Trials that hit the site (`reachable`, `unreachable`).
    TrialsHit,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// Why a property has no rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UnratedReason {
    /// No trial that catalogued the id reported cleanly. This is not a rate of 0%.
    NoRatedTrials,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FirstFailure {
    /// The lowest seed among the failing trials, whatever order trials were added in.
    pub seed: u64,
    pub witness: Option<Witness>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
    /// Inconclusive trials (timed out) whose report showed a failure of this id. The failure is not counted as a
    /// finding, because the report of a killed trial cannot be trusted; this keeps it visible.
    #[serde(default)]
    pub trials_inconclusive_with_failures: u64,
    /// Trials that may back a must-hit claim: rated and not perturbing. UNMET is decided over these only.
    #[serde(default)]
    pub trials_claimable: u64,
    pub trials_hit: u64,
    pub trials_true: u64,
    pub trials_false: u64,
    pub evals: u64,
    pub evals_true: u64,
    pub evals_false: u64,
    pub status: PropertyStatus,
    /// Computed over rated trials only. `None` exactly when `unrated_reason` is `Some`.
    pub rate: Option<Rate>,
    pub unrated_reason: Option<UnratedReason>,
    pub first_failure: Option<FirstFailure>,
}

/// A trial whose declared kind for an id disagreed with the kind first seen in the run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KindMismatch {
    pub id: String,
    pub seed: u64,
    pub kept: AssertKind,
    pub seen: AssertKind,
    /// What that trial's own data says, so a violation there is not lost.
    pub failure: Option<PropertyFailure>,
}

/// Counts across the whole run, separate from any one property.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunSummary {
    pub trials: u64,
    /// Trials containing `eval` lines; exclude them from tier claims.
    pub perturbing_trials: u64,
    pub degraded_trials: u64,
    /// Trials added with [`RunProps::add_inconclusive`]: their failures are not findings.
    #[serde(default)]
    pub inconclusive_trials: u64,
    /// Conclusive trials in which at least one process sent no `end`. Timed-out trials are counted in
    /// `inconclusive_trials` instead.
    pub report_missing_trials: u64,
    /// Id occurrences left out because the run already tracked [`MAX_IDS_PER_RUN`] distinct ids (an id is counted
    /// once for each trial that catalogued it). Each trial that lost an id is also counted in `degraded_trials`.
    #[serde(default)]
    pub ids_dropped: u64,
    /// The distinct-id cap this run used; 0 when read from a report written before it was recorded.
    #[serde(default)]
    pub id_cap: usize,
    /// Trials with a catalog conflict (timed-out trials are not counted: their reports decide nothing).
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
    /// The trial was inconclusive (it timed out): its data is evidence of nothing.
    inconclusive: bool,
}

impl TrialClass {
    fn of(seed: u64, d: &TrialDigest) -> Self {
        TrialClass {
            seed,
            unreported: d.flags.inconclusive || d.flags.report_missing,
            degraded: d.flags.degraded,
            perturbing: d.flags.perturbing,
            inconclusive: d.flags.inconclusive,
        }
    }

    fn is_clean(&self) -> bool {
        !self.unreported && !self.degraded
    }
}

/// One id in one trial, as the fold sees it: the digest entry joined with the catalog row it refers to.
struct Obs<'a> {
    id: &'a str,
    kind: AssertKind,
    msg: &'a str,
    loc: &'a Option<Loc>,
    hits: u64,
    trues: u64,
    falses: u64,
    /// Why the id failed in this trial and how often, when it did.
    failure: Option<(FailureReason, u64)>,
    /// The failure witness the digest carried.
    witness: Option<Witness>,
}

impl Obs<'_> {
    fn failed(&self) -> bool {
        self.failure.is_some()
    }

    /// The failure as a [`PropertyFailure`], using the message and location this trial declared.
    fn property_failure(&self, msg: &str, loc: &Option<Loc>) -> Option<PropertyFailure> {
        let (reason, count) = self.failure?;
        Some(PropertyFailure {
            id: self.id.to_owned(),
            kind: self.kind,
            reason,
            msg: msg.to_owned(),
            loc: loc.clone(),
            count,
            witness: self.witness.clone(),
            dropped: None,
        })
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
    fn start(p: &Obs, decided: bool) -> Self {
        IdAcc {
            kind: p.kind,
            msg: p.msg.to_owned(),
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
    fn decide(self, p: &Obs) -> Self {
        let kept = self.kind == p.kind;
        IdAcc {
            kind: p.kind,
            msg: p.msg.to_owned(),
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

    fn with_trial(mut self, class: TrialClass, p: &Obs) -> Self {
        if class.inconclusive {
            self.trials += 1;
            self.unreported += 1;
            self.perturbing += u64::from(class.perturbing);
            self.inconclusive_failures += u64::from(p.failed());
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
        if p.failed() {
            self.note_failure(class.seed, p.witness.clone());
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

impl PropertyStat {
    /// The rate as text: `k/n (p%) [lower%, upper%]`, with `3/N` when `k` is zero, or
    /// `unrated (0 rated trials)` when no clean trial exists (which is not the same as 0%).
    pub fn rate_text(&self) -> String {
        let Some(r) = self.rate else {
            return "unrated (0 rated trials)".to_owned();
        };
        let pct = |x: f64| format!("{:.1}%", x * 100.0);
        let bound = r
            .rule_of_three
            .map(|b| format!(", 3/N bound {}", pct(b)))
            .unwrap_or_default();
        format!(
            "{}/{} ({}) [{}, {}]{}",
            r.k,
            r.n,
            pct(r.point),
            pct(r.lower),
            pct(r.upper),
            bound
        )
    }
}

/// The counts the fold keeps, apart from the catalog that gives ids their indexes.
#[derive(Debug, Clone)]
struct FoldState {
    summary: RunSummary,
    ids: BTreeMap<String, IdAcc>,
}

/// Accumulator over trials. Consumed and returned by value, so no accumulator is ever shared or mutated in place.
///
/// Trials are folded as [`TrialDigest`]s. [`RunProps::add`] is a convenience that builds the digest (and keeps the
/// catalog it refers to) itself; a receiver that got digests and catalog updates from elsewhere uses
/// [`RunProps::add_digest`] with its own [`RunCatalog`].
#[derive(Debug, Clone)]
pub struct RunProps {
    state: FoldState,
    id_cap: usize,
    catalog: RunCatalog,
}

impl Default for RunProps {
    fn default() -> Self {
        RunProps::with_id_cap(MAX_IDS_PER_RUN)
    }
}

impl RunProps {
    /// An accumulator tracking at most [`MAX_IDS_PER_RUN`] distinct ids.
    pub fn new() -> Self {
        RunProps::default()
    }

    /// An accumulator tracking at most `id_cap` distinct ids. Ids past the cap are counted in
    /// [`RunSummary::ids_dropped`] and the trial that lost them is counted in `degraded_trials`; no per-id statistic
    /// changes. Timed-out trials take cap slots like any other, so ids first seen only in timed-out trials can crowd
    /// out ids that later trials report.
    pub fn with_id_cap(id_cap: usize) -> Self {
        RunProps {
            state: FoldState {
                summary: RunSummary {
                    id_cap,
                    ..RunSummary::default()
                },
                ids: BTreeMap::new(),
            },
            id_cap,
            catalog: RunCatalog::new(),
        }
    }

    /// Fold one trial in. `seed` identifies the trial in first-failure reports.
    #[must_use]
    pub fn add(self, seed: u64, trial: &TrialProps) -> RunProps {
        self.add_classified(seed, trial, false)
    }

    /// Fold in a trial that was inconclusive (it timed out). Its report cannot be trusted, so it is counted as
    /// unreported, adds no hit, true or false counts and no first failure, and cannot make a property a violation;
    /// a failure it showed is only counted in `trials_inconclusive_with_failures`.
    #[must_use]
    pub fn add_inconclusive(self, seed: u64, trial: &TrialProps) -> RunProps {
        self.add_classified(seed, trial, true)
    }

    fn add_classified(self, seed: u64, trial: &TrialProps, inconclusive: bool) -> RunProps {
        let RunProps {
            state,
            id_cap,
            catalog,
        } = self;
        let recorded = catalog.record(trial, inconclusive, id_cap);
        let state = fold_digest(state, seed, &recorded.digest, &recorded.catalog);
        RunProps {
            state,
            id_cap,
            catalog: recorded.catalog,
        }
    }

    /// Fold in a trial that arrived as a digest, against the catalog its indexes refer to. The catalog must already
    /// hold every row the digest uses; entries with an index outside it are ignored. Use this or [`RunProps::add`]
    /// on one accumulator, not both.
    #[must_use]
    pub fn add_digest(self, seed: u64, digest: &TrialDigest, catalog: &RunCatalog) -> RunProps {
        RunProps {
            state: fold_digest(self.state, seed, digest, catalog),
            ..self
        }
    }

    pub fn summary(&self) -> RunSummary {
        self.state.summary.clone()
    }

    /// Per-id statistics sorted by id.
    pub fn finish(self) -> Vec<PropertyStat> {
        self.state
            .ids
            .into_iter()
            .map(|(id, acc)| acc.into_stat(id))
            .collect()
    }
}

fn observe<'a>(e: &DigestEntry, id: &'a str, msg: &'a str, loc: &'a Option<Loc>) -> Obs<'a> {
    let failure = failure_basis(e.kind, e.hits, e.falses);
    Obs {
        id,
        kind: e.kind,
        msg,
        loc,
        hits: e.hits,
        trues: e.trues,
        falses: e.falses,
        failure,
        witness: failure.and(e.witness.clone()),
    }
}

fn fold_digest(
    state: FoldState,
    seed: u64,
    digest: &TrialDigest,
    catalog: &RunCatalog,
) -> FoldState {
    let FoldState {
        mut summary,
        mut ids,
    } = state;
    let class = TrialClass::of(seed, digest);
    let inconclusive = class.inconclusive;
    for e in digest.entries.iter().filter(|e| e.catalogued) {
        let Some(row) = catalog.get(e.idx) else {
            continue;
        };
        let p = observe(e, &row.id, &row.msg, &row.loc);
        let acc = match ids.remove(&row.id) {
            Some(acc) => acc,
            None => IdAcc::start(&p, !inconclusive),
        };
        let acc = match (acc.decided, inconclusive) {
            (false, false) => acc.decide(&p),
            _ => acc,
        };
        let acc = if acc.kind == p.kind {
            acc.with_trial(class, &p)
        } else if inconclusive {
            // A timed-out trial's kind is not trusted: it decides nothing and is no mismatch. It still counts as
            // an unreported trial of the id, but its failure is not attributed to a kind it may not have.
            acc.with_unjudged_trial(class)
        } else {
            summary.kind_mismatches += 1;
            if summary.kind_mismatch_details.len() < MAX_KIND_MISMATCH_DETAILS {
                let (msg, loc) = e
                    .decl
                    .as_ref()
                    .map_or(("", &None), |d| (d.msg.as_str(), &d.loc));
                summary.kind_mismatch_details.push(KindMismatch {
                    id: row.id.clone(),
                    seed,
                    kept: acc.kind,
                    seen: p.kind,
                    failure: p.property_failure(msg, loc),
                });
            }
            acc
        };
        ids.insert(row.id.clone(), acc);
    }
    if !inconclusive {
        for e in digest.entries.iter().filter(|e| e.conflicted) {
            let Some(row) = catalog.get(e.idx) else {
                continue;
            };
            if let Some(acc) = ids.remove(&row.id) {
                ids.insert(row.id.clone(), acc.with_conflict(seed));
            }
        }
    }
    summary.trials += 1;
    summary.perturbing_trials += u64::from(class.perturbing);
    summary.degraded_trials += u64::from(class.degraded || digest.ids_dropped > 0);
    summary.inconclusive_trials += u64::from(inconclusive);
    summary.report_missing_trials += u64::from(class.unreported && !inconclusive);
    summary.conflict_trials += u64::from(!inconclusive && digest.flags.has_conflicts);
    summary.ids_dropped = summary.ids_dropped.saturating_add(digest.ids_dropped);
    FoldState { summary, ids }
}
