//! The hit rate of one run (one arm), as lines of text, from counts only.
//!
//! Rules, each with a test:
//! - A rate is `k/n` with its Wilson 95% interval, never a bare percentage.
//! - The denominator is the rated runs. Inconclusive and crashed runs are listed apart, are never in it, and are never
//!   described as passes.
//! - Nothing rated reads `0/0`, never `0%`.
//! - Zero hits also show the exact one-sided 95% upper bound, and the rule of three where it is close to it.
//! - Excluded runs get a sensitivity line: the rate if every one of them had been a hit.
//! - A run that stopped at its first hit has counts and no rate, because the stop decided how many runs it made.
//!
//! Pure: functions from counts to numbers and strings, with no I/O, so the explorer can use them as they are.

use crate::honesty::{pct, plural};
use crate::stats::{rule_of_three, wilson, zero_event_upper_bound, Z_95};

/// From this many rated runs the rule of three is close enough to the exact bound to be worth quoting beside it.
const RULE_OF_THREE_FROM: u64 = 30;

/// The counts of one run: hits are among the rated runs, and the excluded runs are apart from both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunCounts {
    hits: u64,
    rated: u64,
    inconclusive: u64,
    crashed: u64,
}

/// More hits than rated runs: the counts contradict each other and no rate is computed from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitsExceedRated {
    pub hits: u64,
    pub rated: u64,
}

impl RunCounts {
    /// `rated` is the denominator (it includes the hits); `inconclusive` and `crashed` are runs that were not rated.
    pub fn new(
        hits: u64,
        rated: u64,
        inconclusive: u64,
        crashed: u64,
    ) -> Result<Self, HitsExceedRated> {
        if hits > rated {
            return Err(HitsExceedRated { hits, rated });
        }
        Ok(RunCounts {
            hits,
            rated,
            inconclusive,
            crashed,
        })
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn rated(&self) -> u64 {
        self.rated
    }

    /// Runs left out of the denominator.
    pub fn excluded(&self) -> u64 {
        self.inconclusive.saturating_add(self.crashed)
    }
}

/// `k` of `n` with the Wilson 95% interval, as fractions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Figures {
    pub k: u64,
    pub n: u64,
    pub point: f64,
    pub lower: f64,
    pub upper: f64,
}

/// The figures for `k` of `n`; `None` when `n == 0` or `k > n`.
pub fn figures(k: u64, n: u64) -> Option<Figures> {
    let (lower, upper) = wilson(k, n, Z_95)?;
    Some(Figures {
        k,
        n,
        point: k as f64 / n as f64,
        lower,
        upper,
    })
}

/// The rate if every excluded run had been a hit. `None` when nothing was excluded.
pub fn sensitivity(counts: &RunCounts) -> Option<Figures> {
    let excluded = counts.excluded();
    if excluded == 0 {
        return None;
    }
    figures(
        counts.hits.saturating_add(excluded),
        counts.rated.saturating_add(excluded),
    )
}

fn interval(f: &Figures) -> String {
    format!("Wilson 95% interval [{}, {}]", pct(f.lower), pct(f.upper))
}

fn zero_hit_bounds(n: u64) -> String {
    let exact = zero_event_upper_bound(n).unwrap_or(1.0);
    let base = format!("exact one-sided 95% upper bound {}", pct(exact));
    match rule_of_three(n) {
        Some(three) if n >= RULE_OF_THREE_FROM => {
            format!("{base} (rule of three: {})", pct(three))
        }
        _ => base,
    }
}

fn rated_line(counts: &RunCounts, stopped_at_first_hit: bool) -> String {
    let (k, n) = (counts.hits, counts.rated);
    if n == 0 {
        return "hit runs: 0/0 rated; nothing was rated, so there is no rate".to_owned();
    }
    if stopped_at_first_hit {
        return format!(
            "hit runs: {k} in {} before the run stopped at its first hit; not a rate",
            plural(n, "rated run", "rated runs")
        );
    }
    let Some(f) = figures(k, n) else {
        return format!("hit runs: {k} in {n} rated runs; not a rate");
    };
    if k == 0 {
        return format!(
            "hit runs: 0/{n} rated, {}; {}",
            interval(&f),
            zero_hit_bounds(n)
        );
    }
    format!(
        "hit runs: {k}/{n} rated ({}), {}",
        pct(f.point),
        interval(&f)
    )
}

fn not_rated_line(counts: &RunCounts) -> Option<String> {
    let parts: Vec<String> = [
        (counts.inconclusive, "inconclusive"),
        (counts.crashed, "crashed"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "not rated: {}; reported separately, never in the denominator and never counted as passes",
        parts.join(", ")
    ))
}

fn sensitivity_line(counts: &RunCounts) -> Option<String> {
    let f = sensitivity(counts)?;
    Some(format!(
        "sensitivity: if every non-rated run had been a hit: {}/{} ({}), {}",
        f.k,
        f.n,
        pct(f.point),
        interval(&f)
    ))
}

/// The lines a report prints for one run. `stopped_at_first_hit` is true for a run that ended on its first hit: its
/// counts are shown and nothing is computed from them.
pub fn rate_lines(counts: &RunCounts, stopped_at_first_hit: bool) -> Vec<String> {
    let sensitivity = if stopped_at_first_hit {
        None
    } else {
        sensitivity_line(counts)
    };
    [
        Some(rated_line(counts, stopped_at_first_hit)),
        not_rated_line(counts),
        sensitivity,
    ]
    .into_iter()
    .flatten()
    .collect()
}
