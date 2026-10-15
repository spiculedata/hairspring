//! Trials run in parallel batches, but the result is the same as a sequential run: the lowest failing seed wins.
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::erased::Registry;
use hairspring_engine::report::RunStatus;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
        "hairspring-par-{name}-{}-{nanos}",
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

fn engine(dir: &Path, script: &str, trials: u64, parallelism: u32, timeout: Option<u64>) -> Engine {
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: timeout,
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: trials,
            wall_secs: 120,
        },
    };
    let settings = HermitSettings {
        bin: fake(dir),
        trial_timeout: Duration::from_secs(20),
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
fn trials_in_a_batch_overlap_in_time() {
    let d = scratch("overlap");
    let started = Instant::now();
    let out = engine(&d, "sleep 1; echo fine", 6, 6, None)
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert_eq!(out.trials, 6);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "six 1 s trials in parallel took {:?}",
        started.elapsed()
    );
}

/// Violates when seed % 5 == 3.
const SEEDED_BUG: &str = "[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: balance went negative'; exit 1; }; echo fine";

#[test]
fn the_lowest_failing_seed_is_reported_regardless_of_finish_order() {
    let d = scratch("lowest");
    // Seeds 1..8 in one batch contain two failing seeds (3 and 8); a sequential run would stop at 3.
    let out = engine(&d, SEEDED_BUG, 20, 8, None)
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    let f = out.failure.unwrap();
    assert!(f.message.contains("sched-seed 3 "), "{}", f.message);
    assert_eq!(
        out.trials, 3,
        "reports the same trial count as a sequential run"
    );
}

#[test]
fn a_hung_trial_does_not_hold_up_its_batch_beyond_its_own_timeout() {
    let d = scratch("hung");
    let script = "[ \"$FAKE_SCHED_SEED\" = 2 ] && sleep 30; echo fine";
    let started = Instant::now();
    let out = engine(&d, script, 4, 4, Some(1))
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert_eq!(out.inconclusive, 1);
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn progress_is_reported_per_batch_and_can_cancel() {
    let d = scratch("cancel");
    let mut calls = 0;
    let out = engine(&d, "echo fine", 12, 4, None)
        .run(&RunRequest::new("svc", 1), &mut |p| {
            calls += 1;
            assert!(
                p.trials % 4 == 0 || p.trials == 12,
                "batch-sized progress, got {}",
                p.trials
            );
            calls < 2
        })
        .unwrap();
    assert_eq!(out.error.as_deref(), Some("cancelled"));
    assert!(calls <= 2);
}
