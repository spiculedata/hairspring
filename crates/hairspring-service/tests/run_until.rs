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

fn queued_run(
    svc: &hairspring_service::Service,
    engine: &hairspring_engine::engine::Engine,
) -> String {
    svc.start_run_with_mode(engine, &long_sweep(), RunMode::Sweep)
        .unwrap()
        .id
}

#[test]
fn an_interrupt_never_cancels_a_run_that_belongs_to_someone_else() {
    let (svc, store, engine) = setup();
    // An older queued run, such as one left behind by an earlier command that was killed. It is leased first.
    let foreign = queued_run(&svc, &engine);
    let stop = AtomicBool::new(true);
    let detail = svc
        .run_sync_until(&engine, &long_sweep(), RunMode::Sweep, &stop)
        .unwrap();
    assert_eq!(detail.run.state, RunState::Cancelled);
    let other = svc.get_run(&foreign).unwrap().unwrap().run;
    assert_ne!(other.state, RunState::Cancelled, "{other:?}");
    assert!(!other.state.is_terminal(), "{other:?}");
    let leased = svc
        .lease("later")
        .unwrap()
        .expect("its job is still queued");
    assert_eq!(leased.run_id, foreign);
    assert_eq!(
        store.get_job(&leased.job_id).unwrap().unwrap().attempts,
        1,
        "no attempt was spent"
    );
}

#[test]
fn an_interrupt_cancels_our_run_at_once_when_another_process_holds_its_job() {
    let (svc, _, engine) = setup();
    let run = queued_run(&svc, &engine);
    svc.lease("another-process").unwrap().expect("its job");
    // The job is leased elsewhere, so nothing here can run it: without an interrupt this would wait for it.
    let stop = AtomicBool::new(false);
    let started = std::time::Instant::now();
    let detail = std::thread::scope(|scope| {
        let waiting = scope.spawn(|| svc.finish_run_until(&engine, &run, &stop));
        std::thread::sleep(std::time::Duration::from_millis(300));
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        waiting.join().unwrap().unwrap()
    });
    assert_eq!(detail.run.state, RunState::Cancelled);
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
