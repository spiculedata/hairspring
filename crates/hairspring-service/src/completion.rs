//! How a run ended, as far as its trial budget goes.
//!
//! A passed run that holds fewer trials than its budget is one of two things. Either its caller ended it early, and
//! the trials it never ran cannot back a claim, or it ran out of time or steps, and the trials it did run are all
//! there is: nothing is missing, so a result that is true for those trials stays true and is stated as such.
use hairspring_engine::config::Budget;
use hairspring_store::{RunRecord, RunState};
use serde::Serialize;

/// Where a run stands against its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunCompletion {
    /// Not finished yet.
    Running,
    /// Ran its whole trial budget.
    Complete,
    /// Stopped at its first confirmed failure, so its trial count was decided by the outcome.
    StoppedAtFirstFailure,
    /// Ended on its wall-clock or step limit before reaching the trial budget.
    TimeOrStepLimited { trials_run: u64, trials_budget: u64 },
    /// Ended before its limits by whoever called `complete`, or the trial rows do not match what the worker said it ran.
    EndedEarly,
    /// Ended in an error or was cancelled.
    Aborted,
}

impl RunCompletion {
    /// The label a page shows. Fixed wording; the only variable parts are numbers the server computed.
    pub fn label(&self) -> String {
        match self {
            RunCompletion::Running => "running".to_owned(),
            RunCompletion::Complete => "complete".to_owned(),
            RunCompletion::StoppedAtFirstFailure => "stopped at first failure".to_owned(),
            RunCompletion::TimeOrStepLimited {
                trials_run,
                trials_budget,
            } => format!("time- or step-limited ({trials_run} of {trials_budget} trials)"),
            RunCompletion::EndedEarly => "ended early: trial data is incomplete".to_owned(),
            RunCompletion::Aborted => "aborted".to_owned(),
        }
    }

    /// Whether a must-hit result (UNMET) may be stated for this run.
    pub fn allows_unmet(&self) -> bool {
        matches!(
            self,
            RunCompletion::Complete | RunCompletion::TimeOrStepLimited { .. }
        )
    }
}

/// The trial budget a run was started with, if it was recorded.
fn budget_of(run: &RunRecord) -> Option<Budget> {
    #[derive(serde::Deserialize)]
    struct Payload {
        budget: Option<Budget>,
    }
    serde_json::from_str::<Payload>(&run.config_json)
        .ok()
        .and_then(|p| p.budget)
}

/// Reads a stored run's completion from the flags written when it finished.
pub fn completion_of(run: &RunRecord) -> RunCompletion {
    match run.state {
        RunState::Queued | RunState::Running => return RunCompletion::Running,
        RunState::Error | RunState::Cancelled => return RunCompletion::Aborted,
        RunState::Passed | RunState::Failed => {}
    }
    if run.stopped_at_first_failure {
        return RunCompletion::StoppedAtFirstFailure;
    }
    if run.ingest_incomplete {
        return RunCompletion::EndedEarly;
    }
    match budget_of(run) {
        Some(b) if run.trials < b.max_trials => RunCompletion::TimeOrStepLimited {
            trials_run: run.trials,
            trials_budget: b.max_trials,
        },
        _ => RunCompletion::Complete,
    }
}

/// Whether a run that ended short of its trial budget did so on its wall-clock or step limit.
///
/// Decided here, from the server's own clock, and not from anything the worker says about why it stopped: a worker
/// that simply ends early before either limit is reached has not run out of anything. `started_at` is the time of
/// the run's first lease, so a retried job (a later attempt) is never taken to have run out of time.
pub(crate) fn hit_time_or_step_limit(
    budget: &Budget,
    started_at: Option<i64>,
    now: i64,
    steps: u64,
    attempts: u32,
) -> bool {
    if attempts > 1 {
        return false;
    }
    let wall_ms = i64::try_from(budget.wall_secs.saturating_mul(1000)).unwrap_or(i64::MAX);
    let timed_out = started_at.is_some_and(|s| now.saturating_sub(s) >= wall_ms);
    timed_out || steps >= budget.max_steps
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: Budget = Budget {
        max_steps: 100,
        max_trials: 10,
        wall_secs: 30,
    };

    #[test]
    fn the_wall_clock_limit_is_reached_at_exactly_wall_secs() {
        assert!(!hit_time_or_step_limit(&BUDGET, Some(0), 29_999, 1, 1));
        assert!(hit_time_or_step_limit(&BUDGET, Some(0), 30_000, 1, 1));
    }

    #[test]
    fn the_step_limit_is_reached_at_exactly_max_steps() {
        assert!(!hit_time_or_step_limit(&BUDGET, Some(0), 1, 99, 1));
        assert!(hit_time_or_step_limit(&BUDGET, Some(0), 1, 100, 1));
    }

    #[test]
    fn a_retried_job_is_never_taken_to_have_run_out_of_time() {
        assert!(!hit_time_or_step_limit(&BUDGET, Some(0), 90_000, 1, 2));
    }

    #[test]
    fn an_unknown_start_time_does_not_count_as_time_spent() {
        assert!(!hit_time_or_step_limit(&BUDGET, None, 90_000, 1, 1));
    }

    #[test]
    fn a_clock_that_went_backwards_does_not_count_as_time_spent() {
        assert!(!hit_time_or_step_limit(&BUDGET, Some(90_000), 0, 1, 1));
    }
}
