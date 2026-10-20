//! The hit-rate section of the human-readable run report.
//!
//! The figures and wording come from `hairspring_props::run_rate`; this module only reads the counts off an outcome.
//! Timed out and unusable trials (`inconclusive`) are never rated: they are listed apart and out of the denominator.
use crate::report::{RunOutcome, RunStatus};
use hairspring_props::run_rate::{rate_lines, RunCounts};

/// The counts of an outcome, or `None` for a run that ended in an error (it has no trials to rate).
fn counts(outcome: &RunOutcome) -> Option<RunCounts> {
    if outcome.status == RunStatus::Error {
        return None;
    }
    let hits = u64::from(outcome.status == RunStatus::Failed);
    // A stored outcome whose inconclusive count exceeds its trials is not believed: it rates nothing.
    let rated = outcome.trials.saturating_sub(outcome.inconclusive);
    RunCounts::new(
        hits.min(rated),
        rated,
        outcome.inconclusive.min(outcome.trials),
        0,
    )
    .ok()
}

/// The rate lines, indented under the run's headline; empty when there is nothing to say.
///
/// A run that stopped at its first hit has no rate, so with nothing excluded there is nothing to add to the headline.
pub fn section(outcome: &RunOutcome) -> String {
    let Some(counts) = counts(outcome) else {
        return String::new();
    };
    if outcome.stopped_at_first_failure && counts.excluded() == 0 {
        return String::new();
    }
    rate_lines(&counts, outcome.stopped_at_first_failure)
        .iter()
        .map(|line| format!("  {line}\n"))
        .collect()
}
