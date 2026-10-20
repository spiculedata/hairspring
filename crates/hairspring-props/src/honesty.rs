//! Wording for rates and run headlines, so a reader cannot be told more than the counts support.
//!
//! The rules, each with a test: a rate always shows `k/n` and its interval, never a bare percentage; zero failures show
//! the rule-of-three bound and say what it is; nothing rated is "unrated", never 0%; and a run that stopped at its
//! first confirmed failure has no rate at all, because the stop decided how many trials it ran.

use crate::percent;
use crate::run::{PropertyStat, RateBasis};
use crate::stats::zero_event_upper_bound;
use serde::Serialize;

/// One property's rate, as a page shows it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RateCell {
    /// `k` of `n` rated trials, with the Wilson 95% interval.
    Rate {
        basis: RateBasis,
        k: u64,
        n: u64,
        point: f64,
        lower: f64,
        upper: f64,
    },
    /// No events in `n` rated trials, with the exact one-sided 95% upper bound `1 - 0.05^(1/n)`.
    ZeroEvents {
        basis: RateBasis,
        n: u64,
        upper_bound: f64,
    },
    /// No trial reported cleanly, so there is nothing to rate. This is not 0%.
    Unrated,
    /// The run stopped at its first confirmed failure: the counts are shown and nothing is computed from them.
    NotARate { basis: RateBasis, k: u64, n: u64 },
}

/// The cell for `stat`. `stopped_at_first_failure` applies to every property of such a run, not only the failing one.
pub fn rate_cell(stat: &PropertyStat, stopped_at_first_failure: bool) -> RateCell {
    let Some(r) = stat.rate else {
        return RateCell::Unrated;
    };
    if stopped_at_first_failure {
        return RateCell::NotARate {
            basis: r.basis,
            k: r.k,
            n: r.n,
        };
    }
    match zero_event_upper_bound(r.n) {
        Some(upper_bound) if r.k == 0 => RateCell::ZeroEvents {
            basis: r.basis,
            n: r.n,
            upper_bound,
        },
        _ => RateCell::Rate {
            basis: r.basis,
            k: r.k,
            n: r.n,
            point: r.point,
            lower: r.lower,
            upper: r.upper,
        },
    }
}

pub(crate) fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn events(basis: RateBasis, k: u64) -> String {
    match basis {
        RateBasis::TrialsFalse => plural(k, "failure", "failures"),
        RateBasis::TrialsTrue => plural(k, "trial where it held", "trials where it held"),
        RateBasis::TrialsHit => plural(k, "trial that reached it", "trials that reached it"),
    }
}

impl RateCell {
    pub fn text(&self) -> String {
        match *self {
            RateCell::Rate {
                k,
                n,
                point,
                lower,
                upper,
                ..
            } => format!(
                "{k}/{n} ({}) [{}, {}]",
                percent::nearest(point),
                percent::lower(lower),
                percent::upper(upper)
            ),
            RateCell::ZeroEvents { n, upper_bound, .. } => format!(
                "0/{n}, exact one-sided 95% upper bound {} (binomial, 0 of {n}){}",
                percent::upper(upper_bound),
                three_over_n_note(n)
            ),
            RateCell::Unrated => "unrated (0 rated trials)".to_owned(),
            RateCell::NotARate { basis, k, n } => format!(
                "not a rate (stopped at first failure); {} in {} before stopping",
                events(basis, k),
                plural(n, "rated trial", "rated trials")
            ),
        }
    }
}

/// From this many trials the rule of three is close enough to the exact bound to be worth mentioning beside it.
const THREE_OVER_N_NOTE_FROM: u64 = 30;

/// The rule of three, as a note on the exact bound and never as the bound itself. Below 30 trials it overshoots, so
/// only the exact bound is shown.
pub(crate) fn three_over_n_note(n: u64) -> String {
    if n >= THREE_OVER_N_NOTE_FROM {
        format!("; about 3/{n}")
    } else {
        String::new()
    }
}

/// The bound a zero-failure claim quotes after `rated` rated trials.
pub(crate) fn zero_failure_bound_clause(rated: u64) -> String {
    let bound = zero_event_upper_bound(rated).unwrap_or(1.0);
    format!(
        "exact one-sided 95% upper bound on the failure rate: {} (binomial, 0 of {rated})",
        percent::upper(bound)
    )
}

/// The sentence at the top of a run's property section. It is built from counts only.
///
/// `rated` is the number of rated trials, or `None` for a run stored before it was recorded.
pub fn headline(rated: Option<u64>, violations: u64, first_failing_seed: Option<u64>) -> String {
    if violations > 0 {
        let what = plural(violations, "property failed", "properties failed");
        return match first_failing_seed {
            Some(seed) => format!("{what}; first failing seed {seed}"),
            None => what,
        };
    }
    match rated {
        Some(0) => "Nothing was rated: 0 rated trials".to_owned(),
        Some(n) => format!(
            "No trial failed a property in {}",
            plural(n, "rated trial", "rated trials")
        ),
        None => "No property failure recorded (the rated-trial count is not recorded for this run)"
            .to_owned(),
    }
}

/// How an UNMET result is stated: for the trials that were run, and as a coverage gap rather than a finding.
pub fn unmet_text(claimable_trials: u64) -> String {
    format!(
        "coverage gap, not a finding: never satisfied in {}",
        plural(claimable_trials, "claimable trial", "claimable trials")
    )
}
