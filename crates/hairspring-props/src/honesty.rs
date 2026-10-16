//! Wording for rates and run headlines, so a reader cannot be told more than the counts support.
//!
//! The rules, each with a test: a rate always shows `k/n` and its interval, never a bare percentage; zero failures show
//! the rule-of-three bound and say what it is; nothing rated is "unrated", never 0%; and a run that stopped at its
//! first confirmed failure has no rate at all, because the stop decided how many trials it ran.

use crate::run::{PropertyStat, RateBasis};
use crate::stats::{zero_event_upper_bound, EXACT_BOUND_BELOW};
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
    /// No events in `n` rated trials, with the approximate 95% upper bound `3/n`.
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

fn pct(x: f64) -> String {
    format!("{:.2}%", x.clamp(0.0, 1.0) * 100.0)
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
            } => format!("{k}/{n} ({}) [{}, {}]", pct(point), pct(lower), pct(upper)),
            RateCell::ZeroEvents { n, upper_bound, .. } if n < EXACT_BOUND_BELOW => {
                format!("0/{n}, 95% upper bound {} (exact)", pct(upper_bound))
            }
            RateCell::ZeroEvents { n, upper_bound, .. } => format!(
                "0/{n}, <= 3/{n} ({}) (approx. 95% upper bound)",
                pct(upper_bound)
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

/// The bound a zero-failure claim quotes after `rated` rated trials: exact for few trials, the rule of three after.
pub(crate) fn zero_failure_bound_clause(rated: u64) -> String {
    match zero_event_upper_bound(rated) {
        Some(bound) if rated < EXACT_BOUND_BELOW => {
            format!(
                "95% upper bound on the failure rate: {} (exact)",
                pct(bound)
            )
        }
        _ => format!("approx. 95% upper bound on the failure rate: <= 3/{rated}"),
    }
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
