//! A run driven in the foreground can be stopped from outside: `stop` cancels it, and it ends `cancelled` with what it
//! had stored.
mod common;
use common::*;
use hairspring_engine::config::Budget;
use hairspring_engine::mode::RunMode;
use hairspring_service::StartRun;
use hairspring_store::RunState;
use std::sync::atomic::AtomicBool;

fn long_sweep() -> StartRun {
    StartRun {
        target: "pubsub-atomic".into(),
        seed: Some(1),
        budget: Some(Budget {
            max_steps: 1_000_000,
            max_trials: 1_000_000,
            wall_secs: 120,
        }),
    }
}

#[test]
fn a_run_whose_stop_flag_is_set_ends_cancelled_before_its_budget_is_spent() {
    let (svc, _, engine) = setup();
    let stop = AtomicBool::new(true);
    let detail = svc
        .run_sync_until(&engine, &long_sweep(), RunMode::Sweep, &stop)
        .unwrap();
    assert_eq!(detail.run.state, RunState::Cancelled, "{:?}", detail.run);
    assert!(detail.run.trials < 1_000_000);
}

#[test]
fn a_run_whose_stop_flag_stays_clear_runs_to_its_end() {
    let (svc, _, engine) = setup();
    let stop = AtomicBool::new(false);
    let small = StartRun {
        budget: Some(Budget {
            max_steps: 1_000_000,
            max_trials: 3,
            wall_secs: 60,
        }),
        ..long_sweep()
    };
    let detail = svc
        .run_sync_until(&engine, &small, RunMode::Sweep, &stop)
        .unwrap();
    assert_eq!(detail.run.state, RunState::Passed, "{:?}", detail.run);
}
