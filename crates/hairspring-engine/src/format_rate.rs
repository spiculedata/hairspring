//! The hit-rate section of the human-readable run report.
//!
//! The figures and wording come from `hairspring_props::run_rate`; this module only reads the counts off an outcome.
//! Timed out and unusable trials (`inconclusive`) are never rated: they are listed apart and out of the denominator.
use crate::report::{RunOutcome, RunStatus};
use hairspring_props::run_rate::{rate_lines, RunCounts};

/// Whether the run ended on its first hit. A failed run always did, whatever an older stored record says: its
/// `stopped_at_first_failure` flag defaults to false when the record predates it.
fn stopped_at_first_hit(outcome: &RunOutcome) -> bool {
    outcome.stopped_at_first_failure || outcome.status == RunStatus::Failed
}

/// The counts of an outcome, or `None` when it ran no trials (an error before the first trial has nothing to rate).
/// A run that tested nothing is an error that did run trials, and is counted: it rated none of them.
fn counts(outcome: &RunOutcome) -> Option<RunCounts> {
    if outcome.trials == 0 {
        return None;
    }
    let hits = u64::from(outcome.status == RunStatus::Failed);
    let inconclusive = outcome.inconclusive.min(outcome.trials);
    let rated = outcome.trials - inconclusive;
    RunCounts::new(hits.min(rated), rated, inconclusive, 0).ok()
}

/// Said when a stored count cannot be believed, so the capped figures are not read as exact.
fn capped_note(outcome: &RunOutcome) -> Option<String> {
    (outcome.inconclusive > outcome.trials).then(|| {
        format!(
            "note: the stored inconclusive count ({}) is above the trial count ({}) and was capped; this record may be corrupt",
            outcome.inconclusive, outcome.trials
        )
    })
}

/// The rate lines, indented under the run's headline; empty when there is nothing to say.
///
/// A run that stopped at its first hit has no rate, so with nothing excluded there is nothing to add to the headline.
pub fn section(outcome: &RunOutcome) -> String {
    let Some(counts) = counts(outcome) else {
        return String::new();
    };
    let stopped = stopped_at_first_hit(outcome);
    if stopped && counts.excluded() == 0 {
        return String::new();
    }
    rate_lines(&counts, stopped)
        .into_iter()
        .chain(capped_note(outcome))
        .map(|line| format!("  {line}\n"))
        .collect()
}
