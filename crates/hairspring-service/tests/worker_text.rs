//! Text a worker sends (a failure reason, a progress message, its name) is shown to people and to AI agents, so it is
//! cleaned or refused when it arrives, the same way a finished result is.
use hairspring_engine::backend::Progress;
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_service::{Service, ServiceError, StartRun};
use hairspring_store::{RunState, SqliteStore};
use std::sync::Arc;

const HOSTILE: &str = "a\nb\u{7}c\u{202E}d\u{2028}e\u{2029}f";
const CLEANED: &str = "a b c d e f";

fn boot() -> (Service, Engine) {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    (
        Service::new(Arc::new(SqliteStore::open_in_memory().unwrap())),
        engine,
    )
}

fn start(svc: &Service, engine: &Engine) -> String {
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

#[test]
fn a_failure_reason_from_a_worker_is_one_clean_line_in_the_run_error() {
    let (svc, engine) = boot();
    let run_id = start(&svc, &engine);
    let (worker, _) = svc.register_remote_worker("remote", "{}").unwrap();
    for _ in 0..hairspring_store::MAX_JOB_ATTEMPTS {
        let job = svc.lease(&worker.id).unwrap().expect("a job");
        svc.fail(&job.job_id, &worker.id, HOSTILE).unwrap();
    }
    let run = svc.get_run(&run_id).unwrap().unwrap().run;
    assert_eq!(run.state, RunState::Error);
    assert_eq!(run.error.as_deref(), Some(CLEANED));
}

#[test]
fn a_progress_message_from_a_worker_is_one_clean_line_in_the_stored_event() {
    let (svc, engine) = boot();
    let run_id = start(&svc, &engine);
    let (worker, _) = svc.register_remote_worker("remote", "{}").unwrap();
    let job = svc.lease(&worker.id).unwrap().expect("a job");
    let progress = Progress {
        steps: 1,
        trials: 1,
        message: HOSTILE.to_string(),
    };
    svc.report_job_progress(&job.job_id, &worker.id, &progress)
        .unwrap();
    let events = svc.events(&run_id, 0, 100).unwrap();
    let stored = events
        .iter()
        .find(|e| e.kind == "progress")
        .expect("a progress event");
    let payload: serde_json::Value = serde_json::from_str(&stored.payload_json).unwrap();
    assert_eq!(payload["message"], CLEANED);
}

#[test]
fn a_worker_name_with_a_control_or_invisible_character_is_refused() {
    let (svc, _) = boot();
    for bad in [
        "a\nb",
        "a\u{7}b",
        "a\u{202E}b",
        "a\u{2028}b",
        "a\u{2029}b",
        "a\u{200B}b",
    ] {
        let remote = svc.register_remote_worker(bad, "{}").map(|_| ());
        let local = svc.register_worker(bad, "{}").map(|_| ());
        for result in [remote, local] {
            match result {
                Err(ServiceError::InvalidRequest(m)) => {
                    assert!(m.contains("worker name"), "{m}");
                    assert!(!m.contains(bad), "the refusal repeats the name: {m:?}");
                }
                other => panic!("{bad:?} was not refused: {other:?}"),
            }
        }
    }
    assert!(svc.list_workers().unwrap().is_empty());
    svc.register_remote_worker("Plain worker é 研", "{}")
        .unwrap();
}
