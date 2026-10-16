//! A Hermit-backed run in which every trial is inconclusive ends in an error that says why, not in a validation error.
mod common;

use common::{local, setup};
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::erased::Registry;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings};
use hairspring_service::StartRun;
use hairspring_store::RunState;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const FAKE_HERMIT: &str = r#"#!/bin/sh
shift
while [ "$1" != "--" ]; do shift; done
shift
exec "$@"
"#;

const TRIALS: u64 = 2;

fn hermit_engine(guest: &str) -> Engine {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir: PathBuf =
        std::env::temp_dir().join(format!("hairspring-allinc-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("fake-hermit");
    std::fs::write(&bin, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), guest.into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: Some(1),
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: TRIALS,
            wall_secs: 60,
        },
    };
    let settings = HermitSettings {
        bin,
        trial_timeout: Duration::from_secs(10),
        ..HermitSettings::default()
    };
    Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(settings)))
}

#[test]
fn a_run_where_every_trial_timed_out_is_stored_as_an_error_with_the_nothing_tested_text() {
    let (svc, _, _) = setup();
    let engine = hermit_engine("sleep 30");
    svc.start_run(
        &engine,
        &StartRun {
            target: "svc".into(),
            seed: Some(1),
            budget: None,
        },
    )
    .unwrap();
    let job = svc.lease(&local()).unwrap().expect("job leased");
    let outcome = engine
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();

    svc.complete(&job.job_id, &local(), &outcome)
        .expect("the run is stored, not refused as invalid");

    let run = svc.get_run(&job.run_id).unwrap().unwrap().run;
    assert_eq!(run.state, RunState::Error);
    let stored = run.error.expect("a stored message");
    assert!(
        stored.contains("all 2 trials timed out") && stored.contains("nothing was tested"),
        "{stored}"
    );
    assert!(!stored.contains("invalid request"), "{stored}");
}
