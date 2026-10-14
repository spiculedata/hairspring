use anthill_core::bundle::Bundle;
use anthill_engine::config::{Budget, Config};
use anthill_engine::demos::default_registry;
use anthill_engine::engine::Engine;
use anthill_service::{Service, ServiceError, StartRun};
use anthill_store::{JobStatus, NewRun, RunState, SqliteStore, Store};
use std::sync::Arc;

fn service() -> (Service, Arc<dyn Store>, Engine) {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    (Service::new(store.clone()), store, engine)
}

fn start(target: &str, seed: u64) -> StartRun {
    StartRun {
        target: target.into(),
        seed: Some(seed),
        budget: None,
    }
}

#[test]
fn list_targets_describes_engine_targets() {
    let (svc, _, engine) = service();
    let t = svc.list_targets(&engine);
    let lock = t
        .iter()
        .find(|t| t.name == "lock-lease")
        .expect("lock-lease listed");
    assert_eq!(lock.kind, "sim");
    assert!(!lock.description.is_empty());
}

#[test]
fn start_run_validates_the_target_and_input() {
    let (svc, _, engine) = service();
    assert!(matches!(
        svc.start_run(&engine, &start("nope", 1)),
        Err(ServiceError::InvalidRequest(_))
    ));
    assert!(matches!(
        svc.start_run(&engine, &start("", 1)),
        Err(ServiceError::InvalidRequest(_))
    ));
    let r = svc
        .start_run(
            &engine,
            &StartRun {
                target: "lock-lease".into(),
                seed: None,
                budget: Some(Budget {
                    max_steps: u64::MAX,
                    max_trials: 1,
                    wall_secs: 1,
                }),
            },
        )
        .unwrap();
    assert_eq!(r.state, RunState::Queued);
    assert!(
        r.config_json.contains("50000000"),
        "budget is clamped before it is stored: {}",
        r.config_json
    );
}

#[test]
fn run_sync_stores_failure_steps_and_bundle() {
    let (svc, store, engine) = service();
    let detail = svc.run_sync(&engine, &start("lock-lease", 7)).unwrap();
    assert_eq!(detail.run.state, RunState::Failed);
    assert_eq!(detail.failures.len(), 1);
    let f = svc.get_failure(&detail.failures[0].id).unwrap().unwrap();
    assert!(!f.steps.is_empty() && f.steps.len() == f.record.shrunk_len);
    let text = svc
        .get_bundle(&f.record.bundle_hash)
        .unwrap()
        .expect("bundle blob stored");
    assert_eq!(text, f.bundle);
    assert_eq!(Bundle::decode(&text).unwrap().target, "lock-lease");
    assert_eq!(
        store.get_run(&detail.run.id).unwrap().unwrap().state,
        RunState::Failed
    );
}

#[test]
fn run_sync_passes_the_fixed_target() {
    let (svc, _, engine) = service();
    let d = svc
        .run_sync(
            &engine,
            &StartRun {
                target: "pubsub-atomic".into(),
                seed: Some(1),
                budget: Some(Budget {
                    max_steps: 30_000,
                    max_trials: 3,
                    wall_secs: 30,
                }),
            },
        )
        .unwrap();
    assert_eq!(d.run.state, RunState::Passed);
    assert!(d.failures.is_empty() && d.run.steps > 0);
}

#[test]
fn queued_runs_are_executed_by_workers_and_events_are_recorded() {
    let (svc, store, engine) = service();
    let run = svc.start_run(&engine, &start("pubsub-buggy", 5)).unwrap();
    assert!(
        svc.execute_next(&engine, "w1").unwrap(),
        "a job was processed"
    );
    assert!(
        !svc.execute_next(&engine, "w1").unwrap(),
        "queue is now empty"
    );
    let d = svc.get_run(&run.id).unwrap().unwrap();
    assert_eq!(d.run.state, RunState::Failed);
    let kinds: Vec<_> = svc
        .events(&run.id, 0, 100)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(
        kinds.contains(&"started".to_string()) && kinds.contains(&"finished".to_string()),
        "{kinds:?}"
    );
    let job = store
        .lease_job("probe", 1, anthill_store::now_ms() + 10_000_000)
        .unwrap();
    assert!(job.is_none(), "job is done, nothing re-leasable");
}

#[test]
fn cancelled_runs_are_not_executed() {
    let (svc, _, engine) = service();
    let run = svc.start_run(&engine, &start("lock-lease", 1)).unwrap();
    assert!(svc.cancel_run(&run.id).unwrap());
    assert!(!svc.execute_next(&engine, "w1").unwrap());
    assert_eq!(
        svc.get_run(&run.id).unwrap().unwrap().run.state,
        RunState::Cancelled
    );
}

#[test]
fn engine_errors_become_error_runs_without_retry_storms() {
    let (svc, store, engine) = service();
    let run = store
        .create_run(
            &NewRun {
                target: "ghost".into(),
                backend: "sim".into(),
                seed: 1,
                config_json: "{\"target\":\"ghost\",\"seed\":1,\"budget\":null}".into(),
            },
            1,
        )
        .unwrap();
    let job = store
        .enqueue_job(
            &run.id,
            "run",
            "{\"target\":\"ghost\",\"seed\":1,\"budget\":null}",
            1,
        )
        .unwrap();
    assert!(svc.execute_next(&engine, "w1").unwrap());
    let r = svc.get_run(&run.id).unwrap().unwrap().run;
    assert_eq!(r.state, RunState::Error);
    assert!(r.error.unwrap().contains("ghost"));
    assert_eq!(
        store.get_job(&job.id).unwrap().unwrap().status,
        JobStatus::Done
    );
}

#[test]
fn replay_reproduces_and_rejects_bad_bundles() {
    let (svc, _, engine) = service();
    let d = svc.run_sync(&engine, &start("pubsub-buggy", 5)).unwrap();
    let f = svc.get_failure(&d.failures[0].id).unwrap().unwrap();
    let r = svc.replay_bundle(&engine, &f.bundle).unwrap();
    assert!(r.reproduced && r.steps == f.steps);
    assert!(matches!(
        svc.replay_bundle(&engine, "garbage"),
        Err(ServiceError::InvalidRequest(_))
    ));
    let huge = "x".repeat(2 * 1024 * 1024);
    assert!(matches!(
        svc.replay_bundle(&engine, &huge),
        Err(ServiceError::InvalidRequest(_))
    ));
    let unknown = Bundle::new(1, "x", vec![]).with_target("nope").encode();
    assert!(svc.replay_bundle(&engine, &unknown).is_err());
}

#[test]
fn remote_style_lease_progress_and_complete() {
    let (svc, _, engine) = service();
    let run = svc.start_run(&engine, &start("pubsub-atomic", 3)).unwrap();
    let leased = svc.lease(&engine, "remote-1").unwrap().expect("job leased");
    assert_eq!(
        (leased.run_id.as_str(), leased.target.as_str(), leased.seed),
        (run.id.as_str(), "pubsub-atomic", 3)
    );
    assert!(svc.lease(&engine, "remote-2").unwrap().is_none());
    let outcome = anthill_engine::report::RunOutcome::passed(10, 1, 5);
    assert!(matches!(
        svc.complete(&leased.job_id, "intruder", &outcome),
        Err(ServiceError::InvalidRequest(_))
    ));
    svc.complete(&leased.job_id, "remote-1", &outcome).unwrap();
    assert_eq!(
        svc.get_run(&run.id).unwrap().unwrap().run.state,
        RunState::Passed
    );
}

#[test]
fn several_workers_drain_the_queue_exactly_once() {
    let (svc, _, engine) = service();
    let ids: Vec<_> = (0..6)
        .map(|i| {
            svc.start_run(
                &engine,
                &StartRun {
                    target: "pubsub-atomic".into(),
                    seed: Some(i),
                    budget: Some(Budget {
                        max_steps: 2_000,
                        max_trials: 1,
                        wall_secs: 10,
                    }),
                },
            )
            .unwrap()
            .id
        })
        .collect();
    let handles: Vec<_> = (0..3)
        .map(|w| {
            let (svc, engine) = (svc.clone(), engine.clone());
            std::thread::spawn(move || {
                let mut n = 0;
                while svc.execute_next(&engine, &format!("w{w}")).unwrap() {
                    n += 1;
                }
                n
            })
        })
        .collect();
    let total: i32 = handles.into_iter().map(|h| h.join().unwrap()).sum();
    assert_eq!(total, 6);
    assert!(ids
        .iter()
        .all(|id| svc.get_run(id).unwrap().unwrap().run.state == RunState::Passed));
}

#[test]
fn list_runs_newest_first_and_unknown_run_is_none() {
    let (svc, _, engine) = service();
    let a = svc.start_run(&engine, &start("lock-lease", 1)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    let b = svc.start_run(&engine, &start("lock-lease", 2)).unwrap();
    let ids: Vec<_> = svc
        .list_runs(10)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, vec![b.id, a.id]);
    assert!(svc.get_run("nope").unwrap().is_none());
}

#[test]
fn outcome_is_rebuilt_from_stored_data_for_reporting() {
    let (svc, _, engine) = service();
    let failed = svc.run_sync(&engine, &start("lock-lease", 7)).unwrap();
    let o = svc.outcome(&failed.run.id).unwrap().expect("outcome");
    assert_eq!(o.status, anthill_engine::report::RunStatus::Failed);
    let f = o.failure.expect("failure rebuilt");
    let detail = svc.get_failure(&failed.failures[0].id).unwrap().unwrap();
    assert_eq!((f.steps, f.bundle), (detail.steps, detail.bundle));
    let passed = svc
        .run_sync(
            &engine,
            &StartRun {
                target: "pubsub-atomic".into(),
                seed: Some(1),
                budget: Some(Budget {
                    max_steps: 5_000,
                    max_trials: 1,
                    wall_secs: 10,
                }),
            },
        )
        .unwrap();
    let o = svc.outcome(&passed.run.id).unwrap().unwrap();
    assert_eq!(o.status, anthill_engine::report::RunStatus::Passed);
    assert!(o.failure.is_none() && o.steps_executed > 0);
    assert!(svc.outcome("nope").unwrap().is_none());
}
