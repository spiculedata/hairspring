//! Adapter tests against a fake `hermit` script, so they run anywhere. Tests against the real
//! Hermit live in `real_hermit.rs` and run only when HAIRSPRING_HERMIT_REAL=1 on Linux.
use hairspring_core::bundle::Bundle;
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::erased::Registry;
use hairspring_engine::report::RunStatus;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_engine::EngineError;
use hairspring_hermit::{guest_env, HermitBackend, HermitSettings};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const FAKE_HERMIT: &str = r#"#!/bin/sh
# usage: fake-hermit run [--flag=value ...] -- cmd args...
echo "$@" >> "${FAKE_HERMIT_LOG:-/dev/null}"
echo "HAIRSPRING_SEED=$HAIRSPRING_SEED" >> "${FAKE_HERMIT_LOG:-/dev/null}"
[ "$1" = "run" ] || { echo "fake hermit: unsupported subcommand" >&2; exit 64; }
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
    let dir = std::env::temp_dir().join(format!(
        "hairspring-hermit-{name}-{}-{}",
        std::process::id(),
        hairspring_store_now()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hairspring_store_now() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn write_exe(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn fake(dir: &Path) -> PathBuf {
    write_exe(dir, "fake-hermit", FAKE_HERMIT)
}

fn settings(bin: PathBuf) -> HermitSettings {
    HermitSettings {
        bin,
        trial_timeout: Duration::from_secs(10),
        ..HermitSettings::default()
    }
}

fn engine_for(
    bin: PathBuf,
    name: &str,
    command: &[&str],
    env: &[(&str, &str)],
    budget: Budget,
) -> Engine {
    let spec = TargetSpec {
        name: name.into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: command.iter().map(|s| s.to_string()).collect(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: None,
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
        },
        budget,
    };
    let config = Config {
        targets: BTreeMap::from([(name.to_string(), spec)]),
    };
    Engine::new(Arc::new(Registry::new()), config)
        .with_backend(Arc::new(HermitBackend::new(settings(bin))))
}

fn budget(trials: u64) -> Budget {
    Budget {
        max_steps: 1,
        max_trials: trials,
        wall_secs: 60,
    }
}

/// Violates when seed % 5 == 3 (so the first failing seed after 1 is 3).
const SEEDED_BUG: &str = "[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: balance went negative'; exit 1; }; echo fine seed=$FAKE_SCHED_SEED";

#[test]
fn finds_a_seed_dependent_violation_and_confirms_it_reproduces() {
    let d = scratch("find");
    let e = engine_for(
        fake(&d),
        "svc",
        &["/bin/sh", "-c", SEEDED_BUG],
        &[],
        budget(20),
    );
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    let f = out.failure.unwrap();
    assert_eq!(f.backend, "hermit");
    assert!(
        f.message.contains("sched-seed 3") && f.message.contains("reproduced 3/3"),
        "{}",
        f.message
    );
    assert!(
        f.steps
            .iter()
            .any(|s| s.label.contains("balance went negative")),
        "output excerpt is part of the trace: {:?}",
        f.steps
    );
    let b = Bundle::decode(&f.bundle).unwrap();
    assert_eq!((b.target.as_str(), b.backend.as_str()), ("svc", "hermit"));
    assert_eq!(b.params.get("sched_seed").map(String::as_str), Some("3"));
    assert!(b.params.contains_key("epoch") && b.params.contains_key("output_hash"));
}

#[test]
fn passes_when_no_trial_violates() {
    let d = scratch("pass");
    let e = engine_for(
        fake(&d),
        "svc",
        &["/bin/sh", "-c", "echo fine"],
        &[],
        budget(4),
    );
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Passed);
    assert_eq!(out.trials, 4);
}

#[test]
fn a_nonzero_exit_without_a_marker_is_a_violation() {
    let d = scratch("exit");
    let e = engine_for(
        fake(&d),
        "svc",
        &["/bin/sh", "-c", "echo oops >&2; exit 7"],
        &[],
        budget(3),
    );
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Failed);
    assert!(out
        .failure
        .unwrap()
        .steps
        .iter()
        .any(|s| s.label.contains("exit code 7")));
}

#[test]
fn a_violation_that_does_not_reproduce_is_reported_as_such() {
    let d = scratch("flaky");
    let state = d.join("state");
    let script = "if [ ! -f \"$STATE\" ]; then touch \"$STATE\"; echo 'HAIRSPRING_VIOLATION: once'; exit 1; fi; echo ok";
    let e = engine_for(
        fake(&d),
        "svc",
        &["/bin/sh", "-c", script],
        &[("STATE", state.to_str().unwrap())],
        budget(3),
    );
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    let f = out.failure.expect("still reported");
    assert!(
        f.message.contains("reproduced 0/3")
            && f.message.to_lowercase().contains("not reproducible"),
        "{}",
        f.message
    );
}

#[test]
fn replay_reruns_the_recorded_seed() {
    let d = scratch("replay");
    let e = engine_for(
        fake(&d),
        "svc",
        &["/bin/sh", "-c", SEEDED_BUG],
        &[],
        budget(20),
    );
    let f = e
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap()
        .failure
        .unwrap();
    let bundle = Bundle::decode(&f.bundle).unwrap();
    let r = e.replay(&bundle).unwrap();
    assert!(r.reproduced && !r.steps.is_empty());
    assert!(
        r.steps
            .iter()
            .any(|s| s.label.contains("output matches the recording: yes")),
        "{:?}",
        r.steps
    );
    let fixed = bundle.clone().with_param("sched_seed", "1");
    let r2 = e.replay(&fixed).unwrap();
    assert!(!r2.reproduced, "seed 1 does not violate");
}

#[test]
fn doctor_detects_nondeterministic_programs() {
    let d = scratch("doctor");
    let steady = engine_for(
        fake(&d),
        "svc",
        &["/bin/sh", "-c", "echo seed=$FAKE_SCHED_SEED"],
        &[],
        budget(1),
    );
    assert!(steady.doctor("svc", 3, 0).unwrap().ok);
    let noisy = engine_for(
        fake(&d),
        "svc",
        &["/bin/sh", "-c", "date +%N; od -An -N4 -tu4 /dev/urandom"],
        &[],
        budget(1),
    );
    let r = noisy.doctor("svc", 3, 0).unwrap();
    assert!(!r.ok && r.divergence.unwrap().seed == 1);
}

#[test]
fn hermit_internal_failures_are_errors_not_findings() {
    let d = scratch("infra");
    let broken = write_exe(&d, "broken-hermit", "#!/bin/sh\necho 'HERMIT_INTERNAL_FAILURE class=cli-error' >&2\necho 'Error: failed to initialize ptrace' >&2\nexit 1\n");
    let e = engine_for(broken, "svc", &["/bin/true"], &[], budget(3));
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Error, "{out:?}");
    assert!(out.failure.is_none());
    assert!(out.error.unwrap().contains("ptrace"));
}

#[test]
fn a_missing_hermit_binary_is_a_clear_error() {
    let e = engine_for(
        PathBuf::from("/definitely/not/hermit"),
        "svc",
        &["/bin/true"],
        &[],
        budget(1),
    );
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Error);
    let msg = out.error.unwrap();
    assert!(
        msg.contains("hermit") && msg.to_lowercase().contains("linux"),
        "{msg}"
    );
}

#[test]
fn a_hang_is_a_violation_when_the_target_opts_in() {
    let d = scratch("hang");
    let mut s = settings(fake(&d));
    s.trial_timeout = Duration::from_millis(600);
    let spec_engine = {
        let spec = TargetSpec {
            name: "svc".into(),
            description: String::new(),
            kind: TargetKind::Process {
                command: vec!["/bin/sh".into(), "-c".into(), "sleep 30".into()],
                env: BTreeMap::new(),
                cwd: None,
                hermit_args: vec![],
                trial_timeout_secs: None,
                scheduler: SchedulerSpec::default(),
                hang_is_violation: true,
                parallelism: 1,
            },
            budget: budget(2),
        };
        Engine::new(
            Arc::new(Registry::new()),
            Config {
                targets: BTreeMap::from([("svc".to_string(), spec)]),
            },
        )
        .with_backend(Arc::new(HermitBackend::new(s)))
    };
    let started = std::time::Instant::now();
    let out = spec_engine
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Failed);
    assert!(out
        .failure
        .unwrap()
        .message
        .to_lowercase()
        .contains("timed out"));
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the hung process group was killed, not waited for (took {:?})",
        started.elapsed()
    );
}

#[test]
fn huge_output_is_bounded() {
    let d = scratch("big");
    let e = engine_for(
        fake(&d),
        "svc",
        &[
            "/bin/sh",
            "-c",
            "yes 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' | head -c 6000000; exit 3",
        ],
        &[],
        budget(1),
    );
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    let f = out.failure.unwrap();
    assert!(
        f.steps.len() <= 80
            && f.steps
                .iter()
                .all(|s| s.label.len() <= 400 && s.state.len() <= 4000),
        "trace stays small"
    );
    assert!(f.bundle.len() < 4096);
}

#[test]
fn the_progress_callback_can_stop_exploration() {
    let d = scratch("cancel");
    let e = engine_for(
        fake(&d),
        "svc",
        &["/bin/sh", "-c", "echo fine"],
        &[],
        budget(1000),
    );
    let mut calls = 0;
    let out = e
        .run(&RunRequest::new("svc", 1), &mut |_| {
            calls += 1;
            calls < 3
        })
        .unwrap();
    assert_eq!(out.error.as_deref(), Some("cancelled"));
    assert!(calls <= 3);
}

#[test]
fn hermit_is_invoked_with_a_pinned_epoch_random_scheduling_and_user_args() {
    let d = scratch("argv");
    let log = d.join("argv.log");
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/true".into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec!["--tmp=/x".into()],
            trial_timeout_secs: None,
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
        },
        budget: budget(1),
    };
    let mut s = settings(fake(&d));
    s.extra_env = vec![("FAKE_HERMIT_LOG".into(), log.to_str().unwrap().into())];
    let e = Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(s)));
    e.run(&RunRequest::new("svc", 42), &mut |_| true).unwrap();
    let line = std::fs::read_to_string(&log).unwrap();
    for needle in [
        "run",
        "--epoch=2026-01-01T00:00:00Z",
        "--sched-heuristic=random",
        "--sched-seed=42",
        "--tmp=/x",
        "-- /bin/true",
        "HAIRSPRING_SEED=42",
    ] {
        assert!(line.contains(needle), "missing `{needle}` in `{line}`");
    }
}

#[test]
fn guest_environment_is_minimal_pinned_and_python_aware() {
    std::env::set_var("HAIRSPRING_TEST_SECRET", "leak-me");
    let env = guest_env(
        &["/usr/bin/python3".to_string(), "x.py".to_string()],
        &BTreeMap::from([("MODE".to_string(), "test".to_string())]),
    );
    assert!(
        !env.keys().any(|k| k == "HAIRSPRING_TEST_SECRET"),
        "host environment must not leak into the guest"
    );
    assert_eq!(env.get("MODE").map(String::as_str), Some("test"));
    assert_eq!(
        env.get("PYTHONDONTWRITEBYTECODE").map(String::as_str),
        Some("1"),
        "bytecode caches are mutable host state"
    );
    assert!(
        env.contains_key("PYTHONPYCACHEPREFIX")
            && env.contains_key("PATH")
            && env.get("LANG").map(String::as_str) == Some("C")
    );
    let plain = guest_env(&["/opt/app/server".to_string()], &BTreeMap::new());
    assert!(!plain.contains_key("PYTHONDONTWRITEBYTECODE"));
    let overridden = guest_env(
        &["/usr/bin/python3".to_string()],
        &BTreeMap::from([("PYTHONDONTWRITEBYTECODE".to_string(), "0".to_string())]),
    );
    assert_eq!(
        overridden
            .get("PYTHONDONTWRITEBYTECODE")
            .map(String::as_str),
        Some("0"),
        "explicit config wins"
    );
}

#[test]
fn rejects_non_process_targets() {
    let d = scratch("kind");
    let spec = TargetSpec {
        name: "w".into(),
        description: String::new(),
        kind: TargetKind::Wasm { module: "x".into() },
        budget: budget(1),
    };
    let backend = HermitBackend::new(settings(fake(&d)));
    use hairspring_engine::backend::Backend;
    assert!(!backend.supports(&spec.kind));
    assert!(matches!(
        backend.run(&spec, &RunRequest::new("w", 1), budget(1), &mut |_| true),
        Err(EngineError::BackendUnavailable(_))
    ));
}

#[test]
fn a_target_can_set_its_own_trial_timeout() {
    let d = scratch("pertarget");
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), "sleep 30".into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: Some(1),
            scheduler: SchedulerSpec::default(),
            hang_is_violation: true,
            parallelism: 1,
        },
        budget: budget(1),
    };
    // The backend-wide default is a generous 60s; the target's own 1s limit must win.
    let e = Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(settings(fake(&d)))));
    let started = std::time::Instant::now();
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Failed);
    assert!(out.failure.unwrap().message.contains("timed out after 1s"));
    assert!(
        started.elapsed() < Duration::from_secs(12),
        "took {:?}",
        started.elapsed()
    );
}

// ---- review findings ----
#[test]
fn a_trial_cut_off_by_the_wall_budget_is_inconclusive_not_a_hang() {
    let d = scratch("wallclip");
    let mut s = settings(fake(&d));
    s.trial_timeout = Duration::from_secs(30); // the target's own limit is generous
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), "sleep 30".into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: None,
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: 5,
            wall_secs: 1,
        },
    };
    let e = Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(s)));
    let started = std::time::Instant::now();
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    assert_eq!(
        out.status,
        RunStatus::Error,
        "being out of budget is not a finding, and a run that tested nothing is not a pass: {out:?}"
    );
    assert!(out.failure.is_none());
    let msg = out.error.clone().unwrap();
    assert!(
        msg.contains("wall-clock budget") && msg.contains("nothing was tested"),
        "{msg}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn replay_with_different_behaviour_is_not_a_reproduction() {
    let d = scratch("diffout");
    let e = engine_for(
        fake(&d),
        "svc",
        &["/bin/sh", "-c", SEEDED_BUG],
        &[],
        budget(20),
    );
    let f = e
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap()
        .failure
        .unwrap();
    let tampered = Bundle::decode(&f.bundle)
        .unwrap()
        .with_param("output_hash", "deadbeefdeadbeef");
    let r = e.replay(&tampered).unwrap();
    assert!(
        !r.reproduced,
        "it still fails, but not the way that was recorded"
    );
    assert!(r
        .steps
        .iter()
        .any(|s| s.label.contains("output matches the recording: no")));
}

#[test]
fn a_bare_hermit_name_is_found_on_the_callers_path_not_the_pinned_guest_path() {
    let d = scratch("barepath");
    let name = format!("hairspring-fake-hermit-{}", std::process::id());
    std::fs::copy(fake(&d), d.join(&name)).unwrap();
    let old = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{old}", d.display()));
    let mut s = settings(PathBuf::from(&name));
    s.trial_timeout = Duration::from_secs(10);
    let e = Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([(
                "svc".to_string(),
                TargetSpec {
                    name: "svc".into(),
                    description: String::new(),
                    kind: TargetKind::Process {
                        command: vec!["/bin/sh".into(), "-c".into(), "true".into()],
                        env: BTreeMap::new(),
                        cwd: None,
                        hermit_args: vec![],
                        trial_timeout_secs: None,
                        scheduler: SchedulerSpec::default(),
                        hang_is_violation: false,
                        parallelism: 1,
                    },
                    budget: budget(1),
                },
            )]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(s)));
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    std::env::set_var("PATH", old);
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
}

#[test]
fn processes_left_behind_after_a_normal_exit_are_cleaned_up() {
    let d = scratch("straggler");
    let pidfile = d.join("pid");
    let script = format!("sleep 31 & echo $! > {}", pidfile.display());
    let e = engine_for(fake(&d), "svc", &["/bin/sh", "-c", &script], &[], budget(1));
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Passed);
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let alive = |pid: i32| {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while alive(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !alive(pid),
        "the leftover background process must not outlive the trial"
    );
}

use std::process::Command;

#[test]
fn programs_using_the_former_violation_marker_are_still_understood() {
    let d = scratch("legacy-marker");
    let legacy = "echo 'ANTHILL_VIOLATION: old style'; exit 1";
    let e = engine_for(fake(&d), "svc", &["/bin/sh", "-c", legacy], &[], budget(3));
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    assert!(out.failure.unwrap().message.contains("old style"));
}
