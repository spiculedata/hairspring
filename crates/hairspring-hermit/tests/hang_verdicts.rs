//! A timeout is not a finding by default: it is inconclusive, counted separately, and never a pass.
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::erased::Registry;
use hairspring_engine::report::RunStatus;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const FAKE_HERMIT: &str = r#"#!/bin/sh
shift
seed=0
while [ "$1" != "--" ]; do
  case "$1" in --sched-seed=*) seed=${1#--sched-seed=};; esac
  shift
done
shift
echo "hermit: virtual-time epoch=fake" >&2
FAKE_SCHED_SEED=$seed exec "$@"
"#;

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hairspring-hang-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fake(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("fake-hermit");
    std::fs::write(&p, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn engine(dir: &Path, script: &str, trials: u64, hang_is_violation: bool) -> Engine {
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: Some(1),
            scheduler: SchedulerSpec::default(),
            hang_is_violation,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: trials,
            wall_secs: 60,
        },
    };
    let settings = HermitSettings {
        bin: fake(dir),
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

/// Hangs on odd seeds, fine on even seeds.
const HANGS_ON_ODD: &str = "[ $((FAKE_SCHED_SEED % 2)) -eq 1 ] && sleep 30; echo fine";
/// Hangs on seed 1, violates on seed 2.
const HANG_THEN_VIOLATION: &str = "[ \"$FAKE_SCHED_SEED\" = 1 ] && sleep 30; [ \"$FAKE_SCHED_SEED\" = 2 ] && { echo 'HAIRSPRING_VIOLATION: real bug'; exit 1; }; echo fine";

#[test]
fn timeouts_are_counted_as_inconclusive_not_failures() {
    let d = scratch("mixed");
    let out = engine(&d, HANGS_ON_ODD, 4, false)
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert_eq!((out.trials, out.inconclusive), (4, 2), "{out:?}");
    assert!(out.failure.is_none());
}

#[test]
fn a_run_where_every_trial_timed_out_is_an_error_not_a_pass() {
    let d = scratch("all");
    let out = engine(&d, "sleep 30", 2, false)
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Error, "{out:?}");
    let msg = out.error.unwrap();
    assert!(
        msg.contains("timed out") && msg.contains("nothing was tested"),
        "{msg}"
    );
}

#[test]
fn a_real_violation_after_a_timeout_is_still_found() {
    let d = scratch("then-violation");
    let out = engine(&d, HANG_THEN_VIOLATION, 5, false)
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    assert_eq!(out.inconclusive, 1);
    assert!(out.failure.unwrap().message.contains("real bug"));
}

#[test]
fn a_target_can_declare_that_a_hang_is_the_bug() {
    let d = scratch("promoted");
    let out = engine(&d, "sleep 30", 3, true)
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    assert!(out.failure.unwrap().message.contains("timed out after 1s"));
    assert_eq!(out.inconclusive, 0);
}
