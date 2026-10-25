//! Sweep mode for registry `Sim` targets: every trial of the budget runs, a violation does not end the run.
//!
//! A trial is one exploration round at one seed (the same unit the search loop counts). The rounds share the step
//! budget, as in search, so a target that spends all its steps without a violation runs fewer trials than the trial
//! budget, and the outcome says how many it did run. Only the lowest failing seed is shrunk and reported: shrinking
//! costs far more than a trial, and the other failing seeds are named in the summary and stored as rows.
use crate::backend::{Progress, RunRequest};
use crate::config::Budget;
use crate::erased::Erased;
use crate::report::RunOutcome;
use crate::sim_backend::{failure_report, CANCELLED, ROUND_STEPS};
use crate::sweep::{assemble, assemble_error, SweepEnd};
use crate::trial_log::{ReportedVerdict, TrialReport};
use crate::EngineError;
use hairspring_core::explore::{branching, Config as ExploreConfig};
use std::time::{Duration, Instant};

pub(crate) fn run(
    root: &Erased,
    req: &RunRequest,
    budget: Budget,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<RunOutcome, EngineError> {
    let started = Instant::now();
    let wall = Duration::from_secs(budget.wall_secs);
    let max_trials = budget.max_trials.max(1);
    let mut rows: Vec<TrialReport> = Vec::new();
    let mut failure = None;
    let mut steps_total = 0u64;
    for round in 0..max_trials {
        let remaining = budget.max_steps.saturating_sub(steps_total);
        if round > 0 && (remaining == 0 || started.elapsed() >= wall) {
            break;
        }
        let seed = req.seed.wrapping_add(round);
        let max_steps = ROUND_STEPS.min(remaining).max(1);
        let trial_started = Instant::now();
        let found = branching(
            root,
            &ExploreConfig {
                seed,
                max_steps,
                ..ExploreConfig::default()
            },
        );
        let (verdict, reason) = match &found {
            Some(f) => (
                ReportedVerdict::Violation,
                format!("invariant violated after {} steps", f.steps_executed),
            ),
            None => (ReportedVerdict::Pass, String::new()),
        };
        steps_total += found.as_ref().map_or(max_steps, |f| f.steps_executed);
        if let (Some(f), None) = (&found, &failure) {
            failure = Some(failure_report(root, req, seed, f)?);
        }
        let elapsed_ms = trial_started.elapsed().as_millis() as u64;
        rows.push(TrialReport::without_events(
            round, seed, verdict, &reason, elapsed_ms,
        ));
        let keep_going = progress(Progress {
            steps: steps_total,
            trials: round + 1,
            message: format!("{} of {max_trials} trials run", round + 1),
        });
        if !keep_going {
            return Ok(assemble_error(
                CANCELLED,
                end(rows, failure, steps_total, started),
            ));
        }
    }
    Ok(assemble(end(rows, failure, steps_total, started)))
}

fn end(
    rows: Vec<TrialReport>,
    failure: Option<crate::report::FailureReport>,
    steps_executed: u64,
    started: Instant,
) -> SweepEnd {
    SweepEnd {
        rows,
        failure,
        steps_executed,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}
