//! Sweep mode for WebAssembly targets: every seed of the budget runs, a violation does not end the run.
//!
//! A trial is one run of the module at one seed. Only the lowest failing seed is shrunk and reported: shrinking replays
//! the module thousands of times, far more than a trial costs, and the other failing seeds are named in the summary and
//! stored as rows. A trial that hits the wall-clock limit is inconclusive and ends the sweep: the budget is spent.
use crate::backend::{describe, failure_key, Failing, Loaded, WasmBackend};
use crate::vm::RunResult;
use hairspring_engine::backend::Progress;
use hairspring_engine::config::Budget;
use hairspring_engine::report::{FailureReport, RunOutcome};
use hairspring_engine::sweep::{assemble, assemble_error, SweepEnd};
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use std::time::{Duration, Instant};

const CANCELLED: &str = "cancelled";

fn end(rows: Vec<TrialReport>, failure: Option<FailureReport>, started: Instant) -> SweepEnd {
    SweepEnd {
        steps_executed: rows.len() as u64,
        rows,
        failure,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

pub(crate) fn run(
    backend: &WasmBackend,
    loaded: &Loaded<'_>,
    budget: Budget,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> RunOutcome {
    let started = Instant::now();
    let wall = Duration::from_secs(budget.wall_secs);
    let trials = budget.max_trials.max(1);
    let mut rows: Vec<TrialReport> = Vec::new();
    let mut failure: Option<FailureReport> = None;
    for i in 0..trials {
        if i > 0 && started.elapsed() >= wall {
            break;
        }
        let seed = loaded.req.seed.wrapping_add(i);
        let instantiated =
            loaded
                .module
                .instantiate(seed, Vec::new(), backend.vm_config(loaded.fuel, false));
        let mut vm = match instantiated {
            Ok(vm) => vm,
            Err(e) => return assemble_error(e.to_string(), end(rows, failure, started)),
        };
        let wall_left = wall
            .saturating_sub(started.elapsed())
            .max(Duration::from_secs(1));
        let trial_started = Instant::now();
        let result = vm.run_with_wall(wall_left);
        let elapsed_ms = trial_started.elapsed().as_millis() as u64;
        let row = |verdict, reason: &str| {
            TrialReport::without_events(i, seed, verdict, reason, elapsed_ms)
        };
        if result == RunResult::TimedOut {
            let reason = describe(&result, loaded.fuel);
            rows.push(row(ReportedVerdict::Inconclusive, &reason));
            break;
        }
        match failure_key(&result) {
            None => rows.push(row(ReportedVerdict::Pass, "")),
            Some(key) => {
                rows.push(row(
                    ReportedVerdict::Violation,
                    &describe(&result, loaded.fuel),
                ));
                if failure.is_none() {
                    failure = Some(backend.failure_report(Failing {
                        loaded,
                        seed,
                        key,
                        result,
                        vm,
                        wall_left,
                    }));
                }
            }
        }
        let keep_going = progress(Progress {
            steps: i + 1,
            trials: i + 1,
            message: format!("{} of {trials} seeds explored", i + 1),
        });
        if !keep_going {
            return assemble_error(CANCELLED, end(rows, failure, started));
        }
    }
    assemble(end(rows, failure, started))
}
