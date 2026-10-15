//! A target's scheduler options become Hermit flags, are recorded in bundles, and replay refuses a different strategy.
use hairspring_core::bundle::Bundle;
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::erased::Registry;
use hairspring_engine::scheduler::{Heuristic, SchedulerSpec};
use hairspring_hermit::{HermitBackend, HermitSettings};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const FAKE_HERMIT: &str = r#"#!/bin/sh
echo "$@" >> "${FAKE_HERMIT_LOG:-/dev/null}"
shift
while [ "$1" != "--" ]; do shift; done
shift
echo "hermit: virtual-time epoch=fake" >&2
exec "$@"
"#;

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hairspring-sched-{name}-{}-{nanos}",
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

fn engine(
    dir: &Path,
    command: &[&str],
    scheduler: SchedulerSpec,
    user_args: &[&str],
) -> (Engine, PathBuf) {
    let log = dir.join("argv.log");
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: command.iter().map(|s| s.to_string()).collect(),
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: user_args.iter().map(|s| s.to_string()).collect(),
            trial_timeout_secs: None,
            scheduler,
            hang_is_violation: false,
            parallelism: 1,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: 1,
            wall_secs: 30,
        },
    };
    let settings = HermitSettings {
        bin: fake(dir),
        trial_timeout: Duration::from_secs(10),
        extra_env: vec![("FAKE_HERMIT_LOG".into(), log.to_str().unwrap().into())],
        ..HermitSettings::default()
    };
    let e = Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(settings)));
    (e, log)
}

fn first_argv(log: &Path) -> String {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string()
}

fn chaos_spec() -> SchedulerSpec {
    SchedulerSpec {
        chaos: true,
        max_timeslice_ns: Some(1_000_000),
        skid_margin: Some(25),
        ..SchedulerSpec::default()
    }
}

#[test]
fn the_default_scheduler_is_seeded_random() {
    let d = scratch("default");
    let (e, log) = engine(&d, &["/bin/true"], SchedulerSpec::default(), &[]);
    e.run(&RunRequest::new("svc", 7), &mut |_| true).unwrap();
    let argv = first_argv(&log);
    assert!(
        argv.contains("--sched-heuristic=random") && argv.contains("--sched-seed=7"),
        "{argv}"
    );
    assert!(
        !argv.contains("--chaos") && !argv.contains("--skid-margin"),
        "{argv}"
    );
}

#[test]
fn chaos_options_become_flags_and_no_heuristic_is_passed() {
    let d = scratch("chaos");
    let (e, log) = engine(&d, &["/bin/true"], chaos_spec(), &[]);
    e.run(&RunRequest::new("svc", 3), &mut |_| true).unwrap();
    let argv = first_argv(&log);
    for needle in [
        "--chaos",
        "--max-timeslice=1000000",
        "--skid-margin=25",
        "--sched-seed=3",
    ] {
        assert!(argv.contains(needle), "missing `{needle}` in `{argv}`");
    }
    assert!(!argv.contains("--sched-heuristic"), "{argv}");
}

#[test]
fn sticky_random_passes_the_heuristic_and_its_probability() {
    let d = scratch("sticky");
    let spec = SchedulerSpec {
        heuristic: Some(Heuristic::StickyRandom),
        sticky_permille: Some(900),
        ..SchedulerSpec::default()
    };
    let (e, log) = engine(&d, &["/bin/true"], spec, &[]);
    e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    let argv = first_argv(&log);
    assert!(argv.contains("--sched-heuristic=stickyrandom"), "{argv}");
    assert!(argv.contains("--sched-sticky-random-param=0.900"), "{argv}");
}

#[test]
fn user_hermit_args_come_after_the_scheduler_flags_and_before_the_command() {
    let d = scratch("order");
    let (e, log) = engine(&d, &["/bin/true"], chaos_spec(), &["--tmp=/x"]);
    e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    let argv = first_argv(&log);
    let at = |needle: &str| {
        argv.find(needle)
            .unwrap_or_else(|| panic!("missing `{needle}` in `{argv}`"))
    };
    assert!(
        at("--skid-margin=25") < at("--tmp=/x") && at("--tmp=/x") < at("-- /bin/true"),
        "{argv}"
    );
}

fn failing(dir: &Path, scheduler: SchedulerSpec) -> (Engine, Bundle) {
    let (e, _) = engine(
        dir,
        &["/bin/sh", "-c", "echo 'HAIRSPRING_VIOLATION: boom'; exit 1"],
        scheduler,
        &[],
    );
    let f = e
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap()
        .failure
        .unwrap();
    (e, Bundle::decode(&f.bundle).unwrap())
}

#[test]
fn bundles_record_the_scheduler_flags_that_were_used() {
    let d = scratch("bundle");
    let (_, b) = failing(&d, chaos_spec());
    assert_eq!(
        b.params.get("scheduler").map(String::as_str),
        Some("--chaos --max-timeslice=1000000 --skid-margin=25 --panic-on-rbc-overshoot")
    );
    assert_eq!(b.params.get("heuristic").map(String::as_str), Some("none"));
}

#[test]
fn replay_refuses_a_bundle_recorded_under_a_different_scheduler() {
    let d = scratch("mismatch");
    let (_, bundle) = failing(&d, chaos_spec());
    let (other, _) = engine(
        &scratch("mismatch2"),
        &["/bin/true"],
        SchedulerSpec::default(),
        &[],
    );
    let err = other.replay(&bundle).unwrap_err().to_string();
    assert!(
        err.contains("scheduler") && err.contains("--chaos"),
        "{err}"
    );
}

#[test]
fn replay_under_the_same_scheduler_reproduces() {
    let d = scratch("same");
    let (e, bundle) = failing(&d, chaos_spec());
    assert!(e.replay(&bundle).unwrap().reproduced);
}

#[test]
fn bundles_from_before_scheduler_options_replay_under_the_default() {
    let d = scratch("legacy");
    let (e, bundle) = failing(&d, SchedulerSpec::default());
    let mut legacy = bundle.clone();
    legacy.params.remove("scheduler");
    assert!(e.replay(&legacy).unwrap().reproduced);
}
