//! The read-only view of workers: state, trust, capabilities, what each holds, and the queue behind them.
mod common;

use common::setup;
use hairspring_service::{Service, ServiceConfig, StartRun, WorkerState, MAX_CURRENT_JOBS_SHOWN};

fn queue_run(svc: &Service, engine: &hairspring_engine::engine::Engine) -> String {
    svc.start_run(
        engine,
        &StartRun {
            target: "lock-lease".into(),
            seed: Some(1),
            budget: None,
        },
    )
    .unwrap()
    .id
}

/// The service's lease length, in milliseconds: how long a silent worker still counts as seen.
fn window_ms(_svc: &Service) -> i64 {
    ServiceConfig::default().lease_secs as i64 * 1000
}

#[test]
fn an_empty_server_has_no_workers_and_an_empty_queue() {
    let (svc, _store, _engine) = setup();
    let view = svc.workers_overview_at(1_000).unwrap();
    assert!(view.workers.is_empty());
    assert_eq!((view.queue.queued, view.queue.running), (0, 0));
}

#[test]
fn a_worker_seen_lately_with_no_lease_is_idle_and_one_gone_quiet_is_not_seen_recently() {
    let (svc, store, _engine) = setup();
    let lately = store.register_worker("lately", "{}", 1_000).unwrap();
    let quiet = store.register_worker("quiet", "{}", 1_000).unwrap();
    let now = 1_000 + window_ms(&svc) * 3;
    store.touch_worker(&lately.id, now - 1).unwrap();
    let view = svc.workers_overview_at(now).unwrap();
    let state_of = |name: &str| {
        view.workers
            .iter()
            .find(|w| w.worker.name == name)
            .unwrap()
            .state
    };
    assert_eq!(state_of("lately"), WorkerState::Idle);
    assert_eq!(state_of("quiet"), WorkerState::NotSeenRecently);
    assert_eq!(quiet.last_seen, 1_000);
}

#[test]
fn the_window_edge_counts_as_seen() {
    let (svc, store, _engine) = setup();
    let w = store.register_worker("edge", "{}", 1_000).unwrap();
    let edge = 1_000 + window_ms(&svc);
    let at = |now| svc.workers_overview_at(now).unwrap().workers[0].state;
    assert_eq!(at(edge), WorkerState::Idle);
    assert_eq!(at(edge + 1), WorkerState::NotSeenRecently);
    assert_eq!(w.last_seen, 1_000);
}

#[test]
fn a_worker_holding_a_live_lease_is_busy_and_shows_what_it_holds() {
    let (svc, store, engine) = setup();
    let w = store.register_worker("busy", "{}", 1_000).unwrap();
    let run_id = queue_run(&svc, &engine);
    let job = svc.lease(&w.id).unwrap().expect("a job");
    assert_eq!(job.run_id, run_id);
    let view = svc.workers_overview().unwrap();
    let got = &view.workers[0];
    assert_eq!(got.state, WorkerState::Busy);
    assert_eq!(got.leased_jobs, 1);
    assert_eq!(got.current_jobs.len(), 1);
    assert_eq!(got.current_jobs[0].run_id, run_id);
    assert_eq!(got.current_jobs[0].kind, "run");
    assert_eq!((view.queue.queued, view.queue.running), (0, 1));
}

#[test]
fn an_expired_lease_does_not_make_a_worker_busy() {
    let (svc, store, engine) = setup();
    let w = store.register_worker("late", "{}", 1_000).unwrap();
    queue_run(&svc, &engine);
    let job = store.lease_job(&w.id, 1, 2_000).unwrap().expect("a job");
    assert_eq!(job.status, hairspring_store::JobStatus::Running);
    let view = svc.workers_overview_at(2_000 + 1_001).unwrap();
    assert_eq!(view.workers[0].leased_jobs, 0);
    assert!(view.workers[0].current_jobs.is_empty());
    assert_eq!(view.queue.running, 0);
}

#[test]
fn queue_depth_counts_waiting_jobs_apart_from_held_ones() {
    let (svc, store, engine) = setup();
    let w = store.register_worker("one", "{}", 1_000).unwrap();
    for _ in 0..3 {
        queue_run(&svc, &engine);
    }
    svc.lease(&w.id).unwrap().expect("a job");
    let view = svc.workers_overview().unwrap();
    assert_eq!((view.queue.queued, view.queue.running), (2, 1));
}

#[test]
fn only_the_first_few_held_jobs_are_named_but_all_are_counted() {
    let (svc, store, engine) = setup();
    let w = store.register_worker("greedy", "{}", 1_000).unwrap();
    let total = MAX_CURRENT_JOBS_SHOWN + 2;
    for _ in 0..total {
        queue_run(&svc, &engine);
        svc.lease(&w.id).unwrap().expect("a job");
    }
    let got = &svc.workers_overview().unwrap().workers[0];
    assert_eq!(got.current_jobs.len(), MAX_CURRENT_JOBS_SHOWN);
    assert_eq!(got.leased_jobs as usize, total);
}

#[test]
fn trust_and_capabilities_are_reported_and_bad_capabilities_are_left_out() {
    let (svc, store, _engine) = setup();
    let good = store
        .register_worker("good", "{\"backends\":[\"sim\"]}", 1_000)
        .unwrap();
    store.register_worker("bad", "not json", 1_001).unwrap();
    svc.set_worker_trust(&good.id, true).unwrap();
    let view = svc.workers_overview().unwrap();
    assert!(view.workers[0].worker.trusted);
    assert_eq!(
        view.workers[0].capabilities,
        Some(serde_json::json!({"backends": ["sim"]}))
    );
    assert!(!view.workers[1].worker.trusted);
    assert_eq!(view.workers[1].capabilities, None);
    assert_eq!(view.workers[1].worker.caps_json, "not json");
}

#[test]
fn a_finished_job_leaves_no_busy_worker() {
    let (svc, store, engine) = setup();
    let w = store.register_worker("w", "{}", 1_000).unwrap();
    queue_run(&svc, &engine);
    let job = svc.lease(&w.id).unwrap().expect("a job");
    assert!(store.complete_job(&job.job_id, &w.id).unwrap());
    svc.touch_worker(&w.id).unwrap();
    let view = svc.workers_overview().unwrap();
    assert_eq!(view.workers[0].leased_jobs, 0);
    assert_eq!(view.workers[0].state, WorkerState::Idle);
}
