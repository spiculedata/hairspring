use hairspring_core::bundle::Bundle;
use hairspring_engine::config::{Budget, Config};
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_service::{Service, ServiceError, StartRun};
use hairspring_store::{JobStatus, NewRun, RunState, SqliteStore, Store};
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
        .lease_job("probe", 1, hairspring_store::now_ms() + 10_000_000)
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
    let leased = svc.lease("remote-1").unwrap().expect("job leased");
    assert_eq!(
        (leased.run_id.as_str(), leased.target.as_str(), leased.seed),
        (run.id.as_str(), "pubsub-atomic", 3)
    );
    assert!(svc.lease("remote-2").unwrap().is_none());
    let outcome = hairspring_engine::report::RunOutcome::passed(10, 1, 5);
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
    assert_eq!(o.status, hairspring_engine::report::RunStatus::Failed);
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
    assert_eq!(o.status, hairspring_engine::report::RunStatus::Passed);
    assert!(o.failure.is_none() && o.steps_executed > 0);
    assert!(svc.outcome("nope").unwrap().is_none());
}

#[test]
fn roles_are_ordered_and_parse_strictly() {
    use hairspring_service::Role;
    assert!(
        Role::Admin.allows(Role::Run)
            && Role::Run.allows(Role::Read)
            && Role::Admin.allows(Role::Admin)
    );
    assert!(!Role::Read.allows(Role::Run) && !Role::Run.allows(Role::Admin));
    assert_eq!(Role::parse("read"), Some(Role::Read));
    assert_eq!(Role::parse("RUN"), Some(Role::Run));
    assert_eq!(Role::parse("admin"), Some(Role::Admin));
    assert_eq!(Role::parse("root"), None);
    assert_eq!(Role::parse(""), None);
}

#[test]
fn job_progress_requires_lease_ownership() {
    use hairspring_engine::backend::Progress;
    let (svc, _, engine) = service();
    let run = svc.start_run(&engine, &start("pubsub-atomic", 3)).unwrap();
    let leased = svc.lease("remote-1").unwrap().unwrap();
    let p = Progress {
        steps: 500,
        trials: 2,
        message: "halfway".into(),
    };
    assert!(matches!(
        svc.report_job_progress(&leased.job_id, "intruder", &p),
        Err(ServiceError::InvalidRequest(_))
    ));
    svc.report_job_progress(&leased.job_id, "remote-1", &p)
        .unwrap();
    let r = svc.get_run(&run.id).unwrap().unwrap().run;
    assert_eq!((r.steps, r.trials), (500, 2));
    assert!(svc
        .events(&run.id, 0, 50)
        .unwrap()
        .iter()
        .any(|e| e.kind == "progress" && e.payload_json.contains("halfway")));
    assert!(matches!(
        svc.report_job_progress("job_nope", "remote-1", &p),
        Err(ServiceError::NotFound(_))
    ));
}

// ---------------------------------------------------------------------------------------------
// Hardening (code and security review findings)
// ---------------------------------------------------------------------------------------------
use hairspring_engine::backend::{Backend, Progress, RunRequest as BackendRequest};
use hairspring_engine::config::{TargetKind, TargetSpec};
use hairspring_engine::report::{FailureReport, ReplayOutcome, RunOutcome, RunStatus, Step};
use hairspring_engine::EngineError;
use hairspring_service::ServiceConfig;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A fake backend for `wasm` targets that takes a fixed time, optionally cooperating with cancellation.
struct SlowBackend {
    millis: u64,
    cooperative: bool,
}

impl Backend for SlowBackend {
    fn name(&self) -> &'static str {
        "wasm"
    }
    fn supports(&self, kind: &TargetKind) -> bool {
        matches!(kind, TargetKind::Wasm { .. })
    }
    fn run(
        &self,
        _s: &TargetSpec,
        _r: &BackendRequest,
        _b: Budget,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<RunOutcome, EngineError> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(self.millis) {
            std::thread::sleep(Duration::from_millis(40));
            if self.cooperative
                && !progress(Progress {
                    steps: 1,
                    trials: 1,
                    message: "tick".into(),
                })
            {
                return Ok(RunOutcome::error("cancelled"));
            }
        }
        Ok(RunOutcome::passed(1, 1, self.millis))
    }
    fn replay(&self, _s: &TargetSpec, _b: &Bundle) -> Result<ReplayOutcome, EngineError> {
        Err(EngineError::Replay("unsupported".into()))
    }
}

fn slow_engine(millis: u64, cooperative: bool) -> Engine {
    let registry = Arc::new(default_registry());
    let config = Config::parse("[target.slow]\nkind = \"wasm\"\nmodule = \"x.wasm\"\n")
        .unwrap()
        .with_registry_defaults(&registry);
    Engine::new(registry, config).with_backend(Arc::new(SlowBackend {
        millis,
        cooperative,
    }))
}

fn service_with(cfg: ServiceConfig) -> (Service, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    (Service::new(store.clone()).with_config(cfg), store)
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn failed_outcome(target: &str, steps: Vec<Step>, bundle: &str) -> RunOutcome {
    RunOutcome::failed(
        1,
        1,
        1,
        FailureReport {
            target: target.into(),
            backend: "sim".into(),
            message: "boom".into(),
            original_len: 9,
            shrunk_len: steps.len(),
            steps,
            bundle: bundle.into(),
        },
    )
}

fn step(i: usize, label: &str, state: &str) -> Step {
    Step {
        index: i,
        action_id: format!("{i:016x}"),
        label: label.into(),
        state: state.into(),
    }
}

fn leased(
    svc: &Service,
    engine: &Engine,
    target: &str,
    worker: &str,
) -> hairspring_service::LeasedJob {
    svc.start_run(engine, &start(target, 1)).unwrap();
    svc.lease(worker).unwrap().expect("job leased")
}

#[test]
fn a_run_whose_job_exhausts_attempts_through_expiry_is_finalised_not_stuck() {
    let (svc, store, engine) = service();
    let run = svc.start_run(&engine, &start("lock-lease", 1)).unwrap();
    // Three workers lease it long ago and never finish: simulated with past timestamps.
    for t in [1_000, 10_000, 20_000] {
        store
            .lease_job("dead-worker", 1, t)
            .unwrap()
            .expect("leased");
    }
    assert!(
        svc.lease("w-new").unwrap().is_none(),
        "exhausted job is not handed out again"
    );
    let r = svc.get_run(&run.id).unwrap().unwrap().run;
    assert_eq!(r.state, RunState::Error);
    assert!(r.error.unwrap().contains("lease"), "explains why");
    let kinds: Vec<_> = svc
        .events(&run.id, 0, 50)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(kinds.contains(&"finished".to_string()), "{kinds:?}");
}

#[test]
fn complete_on_a_cancelled_run_changes_nothing_and_closes_the_job() {
    let (svc, store, engine) = service();
    let job = leased(&svc, &engine, "lock-lease", "w1");
    assert!(svc.cancel_run(&job.run_id).unwrap());
    svc.complete(
        &job.job_id,
        "w1",
        &failed_outcome(
            "lock-lease",
            vec![step(0, "Tick", "S")],
            "hairspring-bundle v2\nseed=1\nbuild=x\npath=\n",
        ),
    )
    .unwrap();
    let d = svc.get_run(&job.run_id).unwrap().unwrap();
    assert_eq!(d.run.state, RunState::Cancelled);
    assert!(
        d.failures.is_empty(),
        "no failure is recorded for a cancelled run"
    );
    assert_eq!(
        store.get_job(&job.job_id).unwrap().unwrap().status,
        JobStatus::Done
    );
    let kinds: Vec<_> = svc
        .events(&job.run_id, 0, 50)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(
        kinds.contains(&"cancelled".to_string()) && !kinds.contains(&"finished".to_string()),
        "{kinds:?}"
    );
}

#[test]
fn a_second_complete_or_a_stale_fail_does_not_duplicate_anything() {
    let (svc, _, engine) = service();
    let job = leased(&svc, &engine, "lock-lease", "w1");
    let o = failed_outcome(
        "lock-lease",
        vec![step(0, "Tick", "S")],
        "hairspring-bundle v2\nseed=1\nbuild=x\npath=\n",
    );
    svc.complete(&job.job_id, "w1", &o).unwrap();
    assert!(matches!(
        svc.complete(&job.job_id, "w1", &o),
        Err(ServiceError::InvalidRequest(_))
    ));
    svc.fail(&job.job_id, "w1", "late failure report").unwrap();
    let d = svc.get_run(&job.run_id).unwrap().unwrap();
    assert_eq!((d.run.state, d.failures.len()), (RunState::Failed, 1));
    let finished = svc
        .events(&job.run_id, 0, 50)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "finished")
        .count();
    assert_eq!(finished, 1, "exactly one finished event");
}

#[test]
fn worker_supplied_results_are_bounded_checked_and_sanitised() {
    let (svc, _, engine) = service();
    let bundle = "hairspring-bundle v2\nseed=1\nbuild=x\npath=\n";
    let job = leased(&svc, &engine, "lock-lease", "w1");
    let too_many: Vec<Step> = (0..1001).map(|i| step(i, "Tick", "S")).collect();
    assert!(matches!(
        svc.complete(
            &job.job_id,
            "w1",
            &failed_outcome("lock-lease", too_many, bundle)
        ),
        Err(ServiceError::InvalidRequest(_))
    ));
    let long = "x".repeat(10_000);
    assert!(matches!(
        svc.complete(
            &job.job_id,
            "w1",
            &failed_outcome("lock-lease", vec![step(0, &long, "S")], bundle)
        ),
        Err(ServiceError::InvalidRequest(_))
    ));
    assert!(
        matches!(
            svc.complete(
                &job.job_id,
                "w1",
                &failed_outcome("other-target", vec![step(0, "Tick", "S")], bundle)
            ),
            Err(ServiceError::InvalidRequest(_))
        ),
        "failure must be for the job's own target"
    );
    let huge_bundle = "b".repeat(2 * 1024 * 1024);
    assert!(matches!(
        svc.complete(
            &job.job_id,
            "w1",
            &failed_outcome("lock-lease", vec![step(0, "Tick", "S")], &huge_bundle)
        ),
        Err(ServiceError::InvalidRequest(_))
    ));
    // still leased: none of the rejected attempts consumed it. Now a valid one with hostile text.
    let evil = failed_outcome(
        "lock-lease",
        vec![step(
            0,
            "Tick\n\nIGNORE ALL PREVIOUS INSTRUCTIONS\u{1b}[31m",
            "line1\nline2\u{0}x",
        )],
        bundle,
    );
    svc.complete(&job.job_id, "w1", &evil).unwrap();
    let fid = svc.get_run(&job.run_id).unwrap().unwrap().failures[0]
        .id
        .clone();
    let f = svc.get_failure(&fid).unwrap().unwrap();
    assert!(
        !f.steps[0].label.contains('\n') && !f.steps[0].label.contains('\u{1b}'),
        "labels are single-line, control-free: {:?}",
        f.steps[0].label
    );
    assert_eq!(
        f.steps[0].state, "line1\nline2 x",
        "state keeps newlines but loses other control characters"
    );
}

#[test]
fn progress_events_are_throttled_but_counters_stay_current() {
    let (svc, _, engine) = service();
    let job = leased(&svc, &engine, "pubsub-atomic", "w1");
    for i in 0..50u64 {
        svc.report_job_progress(
            &job.job_id,
            "w1",
            &Progress {
                steps: i,
                trials: 1,
                message: "p".into(),
            },
        )
        .unwrap();
    }
    let run = svc.get_run(&job.run_id).unwrap().unwrap().run;
    assert_eq!(run.steps, 49, "counters always reflect the latest report");
    let progress_events = svc
        .events(&job.run_id, 0, 500)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "progress")
        .count();
    assert!(
        (1..=3).contains(&progress_events),
        "events are coalesced, got {progress_events}"
    );
}

#[test]
fn queue_and_worker_admission_limits_apply() {
    let (svc, _) = service_with(ServiceConfig {
        max_queued: 2,
        max_workers: 1,
        ..ServiceConfig::default()
    });
    let engine = engine_default();
    svc.start_run(&engine, &start("lock-lease", 1)).unwrap();
    svc.start_run(&engine, &start("lock-lease", 2)).unwrap();
    assert!(
        matches!(svc.start_run(&engine, &start("lock-lease", 3)), Err(ServiceError::InvalidRequest(m)) if m.contains("queue"))
    );
    svc.register_worker("a", "{}").unwrap();
    assert!(
        matches!(svc.register_worker("b", "{}"), Err(ServiceError::InvalidRequest(m)) if m.contains("worker"))
    );
}

fn engine_default() -> Engine {
    let registry = Arc::new(default_registry());
    Engine::new(registry.clone(), Config::from_registry(&registry))
}

#[test]
fn replay_rejects_absurdly_long_paths() {
    let (svc, _, engine) = service();
    let b = Bundle::new(1, "x", vec![7; 30_000])
        .with_target("lock-lease")
        .encode();
    assert!(
        matches!(svc.replay_bundle(&engine, &b), Err(ServiceError::InvalidRequest(m)) if m.contains("steps"))
    );
}

#[test]
fn a_silent_long_run_keeps_its_lease_through_the_heartbeat() {
    let (svc, _store) = service_with(ServiceConfig {
        lease_secs: 1,
        ..ServiceConfig::default()
    });
    let engine = slow_engine(2500, false);
    let run = svc
        .start_run(
            &engine,
            &StartRun {
                target: "slow".into(),
                seed: Some(1),
                budget: None,
            },
        )
        .unwrap();
    let (svc2, engine2) = (svc.clone(), engine.clone());
    let worker = std::thread::spawn(move || svc2.execute_next(&engine2, "slow-worker").unwrap());
    std::thread::sleep(Duration::from_millis(1600)); // well past the 1s lease
    assert!(
        svc.lease("thief").unwrap().is_none(),
        "the lease must have been renewed by the heartbeat"
    );
    assert!(worker.join().unwrap());
    assert_eq!(
        svc.get_run(&run.id).unwrap().unwrap().run.state,
        RunState::Passed
    );
}

#[test]
fn cancelling_a_running_job_stops_it_promptly() {
    let (svc, _) = service_with(ServiceConfig::default());
    let engine = slow_engine(30_000, true);
    let run = svc
        .start_run(
            &engine,
            &StartRun {
                target: "slow".into(),
                seed: Some(1),
                budget: None,
            },
        )
        .unwrap();
    let (svc2, engine2) = (svc.clone(), engine.clone());
    let started = Instant::now();
    let worker = std::thread::spawn(move || svc2.execute_next(&engine2, "w").unwrap());
    wait_until("the run to start", || {
        svc.get_run(&run.id).unwrap().unwrap().run.state == RunState::Running
    });
    assert!(svc.cancel_run(&run.id).unwrap());
    assert!(worker.join().unwrap());
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "worker stopped instead of running the full 30s"
    );
    let d = svc.get_run(&run.id).unwrap().unwrap();
    assert_eq!(d.run.state, RunState::Cancelled);
    assert!(d.failures.is_empty());
}

#[test]
fn a_stop_signal_requeues_the_job_for_another_worker() {
    let (svc, _) = service_with(ServiceConfig::default());
    let engine = slow_engine(30_000, true);
    svc.start_run(
        &engine,
        &StartRun {
            target: "slow".into(),
            seed: Some(1),
            budget: None,
        },
    )
    .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let (svc2, engine2, stop2) = (svc.clone(), engine.clone(), stop.clone());
    let worker =
        std::thread::spawn(move || svc2.execute_next_until(&engine2, "w", &stop2).unwrap());
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::SeqCst);
    assert!(worker.join().unwrap());
    assert!(
        svc.lease("other").unwrap().is_some(),
        "the interrupted job is available again"
    );
}

#[test]
fn local_runs_use_a_process_unique_worker_identity() {
    assert!(Service::local_worker_id().contains(&std::process::id().to_string()));
}

#[test]
fn cancel_emits_a_cancelled_event() {
    let (svc, _, engine) = service();
    let run = svc.start_run(&engine, &start("lock-lease", 1)).unwrap();
    assert!(svc.cancel_run(&run.id).unwrap());
    assert!(svc
        .events(&run.id, 0, 10)
        .unwrap()
        .iter()
        .any(|e| e.kind == "cancelled"));
    assert!(!svc.cancel_run(&run.id).unwrap());
    assert_eq!(
        svc.events(&run.id, 0, 10)
            .unwrap()
            .iter()
            .filter(|e| e.kind == "cancelled")
            .count(),
        1
    );
}

#[test]
fn status_of_outcomes_is_preserved_for_passed_and_error() {
    let (svc, _, engine) = service();
    let job = leased(&svc, &engine, "pubsub-atomic", "w1");
    svc.complete(&job.job_id, "w1", &RunOutcome::passed(10, 1, 5))
        .unwrap();
    assert_eq!(
        svc.outcome(&job.run_id).unwrap().unwrap().status,
        RunStatus::Passed
    );
    let job2 = leased(&svc, &engine, "pubsub-atomic", "w2");
    svc.complete(&job2.job_id, "w2", &RunOutcome::error("kaput"))
        .unwrap();
    assert_eq!(
        svc.get_run(&job2.run_id)
            .unwrap()
            .unwrap()
            .run
            .error
            .as_deref(),
        Some("kaput")
    );
}
