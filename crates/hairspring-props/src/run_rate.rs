//! The hit rate of one job (one arm), as lines of text, from counts only.
//!
//! What is counted is trials: a job runs many trials, and a hit is a trial in which a property failed. Rules, each
//! with a test:
//! - A rate is `k/n` rated trials with its Wilson 95% interval, never a bare percentage.
//! - The denominator is the rated trials. Inconclusive and crashed trials are listed apart, are never in it, and are
//!   never described as passes.
//! - Nothing rated reads `0/0`, never `0%`, and has no sensitivity line.
//! - Zero hits show one bound, the exact one-sided 95% upper bound (with the rule of three as a note from 30 trials,
//!   as `honesty` words it). The two-sided Wilson interval is shown only when there is a hit, so two different 95%
//!   bounds never sit side by side.
//! - Excluded trials get a sensitivity line: the rate if every one of them had been a hit.
//! - A job that stopped at its first hit has counts and no rate, because the stop decided how many trials it ran.
//!
//! Percentages come from [`crate::percent`] (upper bounds round up) and the zero-hit wording from [`crate::honesty`],
//! so these lines and the property cells cannot drift apart. Pure: no I/O, so the explorer can use them as they are.

use crate::honesty::{plural, three_over_n_note};
use crate::percent;
use crate::stats::{wilson, zero_event_upper_bound, Z_95};

/// The counts of one job: hits are among the rated trials, and the excluded trials are apart from both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunCounts {
    hits: u64,
    rated: u64,
    inconclusive: u64,
    crashed: u64,
}

/// More hits than rated trials: the counts contradict each other and no rate is computed from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitsExceedRated {
    pub hits: u64,
    pub rated: u64,
}

impl RunCounts {
    /// `rated` is the denominator (it includes the hits); `inconclusive` and `crashed` are trials that were not rated.
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

    /// Trials left out of the denominator.
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

/// The rate if every excluded trial had been a hit. `None` when nothing was excluded or nothing was rated (with
/// nothing rated the answer would only say "all of them", which is not information).
pub fn sensitivity(counts: &RunCounts) -> Option<Figures> {
    let excluded = counts.excluded();
    if excluded == 0 || counts.rated == 0 {
        return None;
    }
    figures(
        counts.hits.saturating_add(excluded),
        counts.rated.saturating_add(excluded),
    )
}

fn with_point(f: &Figures) -> String {
    format!(
        "{}/{} ({}), Wilson 95% interval [{}, {}]",
        f.k,
        f.n,
        percent::nearest(f.point),
        percent::lower(f.lower),
        percent::upper(f.upper)
    )
}

fn zero_hit_text(n: u64) -> String {
    let exact = zero_event_upper_bound(n).unwrap_or(1.0);
    format!(
        "0/{n} rated trials, exact one-sided 95% upper bound {} (binomial, 0 of {n}){}",
        percent::upper(exact),
        three_over_n_note(n)
    )
}

fn rated_line(counts: &RunCounts, stopped_at_first_hit: bool) -> String {
    let (k, n) = (counts.hits, counts.rated);
    if n == 0 {
        return "hits: 0/0 rated trials; nothing was rated, so there is no rate".to_owned();
    }
    if stopped_at_first_hit {
        return format!(
            "hits: {k} in {} before the run stopped at its first hit; not a rate",
            plural(n, "rated trial", "rated trials")
        );
    }
    if k == 0 {
        return format!("hits: {}", zero_hit_text(n));
    }
    match figures(k, n) {
        Some(f) => format!(
            "hits: {k}/{n} rated trials ({}), {}",
            percent::nearest(f.point),
            interval_text(&f)
        ),
        None => format!("hits: {k} in {n} rated trials; not a rate"),
    }
}

fn interval_text(f: &Figures) -> String {
    format!(
        "Wilson 95% interval [{}, {}]",
        percent::lower(f.lower),
        percent::upper(f.upper)
    )
}

fn not_rated_line(counts: &RunCounts) -> Option<String> {
    let parts: Vec<String> = [
        (counts.inconclusive, "inconclusive"),
        (counts.crashed, "failed in the host"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "not rated trials: {}; reported separately, never in the denominator and never counted as passes",
        parts.join(", ")
    ))
}

fn sensitivity_line(counts: &RunCounts) -> Option<String> {
    let f = sensitivity(counts)?;
    Some(format!(
        "sensitivity: if every non-rated trial had been a hit: {}",
        with_point(&f)
    ))
}

/// The lines a report prints for one job. `stopped_at_first_hit` is true for a job that ended on its first hit: its
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
