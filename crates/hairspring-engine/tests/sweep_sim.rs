//! Sweep mode on the simulation backend: every trial of the budget runs, whatever the earlier ones did.
use hairspring_core::bundle::Bundle;
use hairspring_core::demo::LockLease;
use hairspring_core::explore::{branching, Config as ExploreConfig};
use hairspring_engine::config::{Budget, Config};
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::mode::RunMode;
use hairspring_engine::report::{RunOutcome, RunStatus};
use hairspring_engine::trial_log::ReportedVerdict;
use std::sync::Arc;

const SEED: u64 = 7;
/// The simulation backend's own step allowance per trial.
const ROUND_STEPS: u64 = 100_000;

fn engine() -> Engine {
    let registry = Arc::new(default_registry());
    let config = Config::from_registry(&registry);
    Engine::new(registry, config)
}

fn budget(max_trials: u64) -> Budget {
    Budget {
        max_steps: 10_000_000,
        max_trials,
        wall_secs: 60,
    }
}

fn sweep(target: &str, trials: u64) -> RunOutcome {
    engine()
        .run(
            &RunRequest::new(target, SEED)
                .with_budget(budget(trials))
                .with_mode(RunMode::Sweep),
            &mut |_| true,
        )
        .unwrap()
}

/// Whether one trial of `lock-lease` at `seed` finds the bug, worked out without the engine.
fn violates(seed: u64) -> bool {
    branching(
        &LockLease::new(),
        &ExploreConfig {
            seed,
            max_steps: ROUND_STEPS,
            ..ExploreConfig::default()
        },
    )
    .is_some()
}

#[test]
fn a_sweep_runs_every_trial_and_stores_one_row_each_with_consecutive_seeds() {
    let out = sweep("lock-lease", 6);
    assert_eq!(out.trials, 6, "{out:?}");
    assert_eq!(out.trial_log.len(), 6);
    for (i, row) in out.trial_log.iter().enumerate() {
        assert_eq!((row.seq, row.seed), (i as u64, SEED + i as u64));
        let expected = if violates(row.seed) {
            ReportedVerdict::Violation
        } else {
            ReportedVerdict::Pass
        };
        assert_eq!(row.verdict, expected, "seed {}", row.seed);
    }
}

#[test]
fn a_sweep_counts_its_violations_and_reports_the_lowest_failing_seed() {
    let out = sweep("lock-lease", 6);
    let violating: Vec<u64> = (SEED..SEED + 6).filter(|s| violates(*s)).collect();
    assert!(!violating.is_empty(), "the fixture must fail at some seed");
    let summary = out.sweep.as_ref().expect("a sweep carries its summary");
    assert_eq!(summary.violations, violating.len() as u64);
    assert_eq!(summary.passes, 6 - violating.len() as u64);
    assert_eq!(summary.first_failing_seeds, violating);
    assert_eq!(out.status, RunStatus::Failed);
    assert!(!out.stopped_at_first_failure, "a sweep does not stop");
    let failure = out.failure.expect("the lowest failing seed has a report");
    assert_eq!(Bundle::decode(&failure.bundle).unwrap().seed, violating[0]);
}

#[test]
fn a_sweep_is_deterministic_for_a_seed() {
    // Everything but the clock: elapsed time is informational and differs between identical runs.
    let stable = |out: RunOutcome| {
        let rows: Vec<_> = out
            .trial_log
            .iter()
            .map(|r| (r.seq, r.seed, r.verdict, r.reason.clone()))
            .collect();
        (out.status, out.steps_executed, out.failure, out.sweep, rows)
    };
    assert_eq!(
        stable(sweep("lock-lease", 4)),
        stable(sweep("lock-lease", 4))
    );
}

#[test]
fn search_stays_the_default_and_stops_at_its_first_failure() {
    let out = engine()
        .run(&RunRequest::new("lock-lease", SEED), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Failed);
    assert!(out.stopped_at_first_failure);
    assert!(out.sweep.is_none() && out.trial_log.is_empty());
    assert_eq!(RunRequest::new("lock-lease", SEED).mode, RunMode::Search);
}

#[test]
fn a_sweep_ends_at_the_step_budget_and_says_how_many_trials_it_ran() {
    let req = RunRequest::new("pubsub-atomic", 1)
        .with_budget(Budget {
            max_steps: 50_000,
            max_trials: 10,
            wall_secs: 30,
        })
        .with_mode(RunMode::Sweep);
    let out = engine().run(&req, &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert!(out.trials >= 1 && out.trials < 10, "{}", out.trials);
    assert_eq!(out.trial_log.len() as u64, out.trials);
    assert_eq!(out.sweep.unwrap().passes, out.trials);
}

#[test]
fn a_sweep_can_be_cancelled_between_trials() {
    let mut calls = 0;
    let out = engine()
        .run(
            &RunRequest::new("lock-lease", SEED)
                .with_budget(budget(50))
                .with_mode(RunMode::Sweep),
            &mut |_| {
                calls += 1;
                calls < 3
            },
        )
        .unwrap();
    assert_eq!(out.status, RunStatus::Error);
    assert_eq!(out.error.as_deref(), Some("cancelled"));
    assert_eq!(calls, 3);
    assert_eq!(out.trials, 3, "the trials judged before the stop are kept");
}

#[test]
fn a_budget_of_zero_trials_still_runs_one() {
    assert_eq!(sweep("lock-lease", 0).trials, 1);
}
