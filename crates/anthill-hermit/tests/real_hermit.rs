//! Tests against the REAL Hermit. They run only when `ANTHILL_HERMIT_REAL=1` on an x86-64 Linux
//! machine with hermit installed (set `ANTHILL_HERMIT` if it is not on the PATH, and
//! `ANTHILL_REAL_WORKDIR` to a directory outside /tmp and your home; the default is /opt/anthill-real).
use anthill_core::bundle::Bundle;
use anthill_engine::config::{Budget, Config, TargetKind, TargetSpec};
use anthill_engine::engine::{Engine, RunRequest};
use anthill_engine::erased::Registry;
use anthill_engine::report::RunStatus;
use anthill_hermit::{HermitBackend, HermitSettings};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

const BUGGY: &str = include_str!("fixtures/double_init.c");
const FIXED: &str = include_str!("fixtures/single_init.c");

fn enabled() -> bool {
    std::env::var("ANTHILL_HERMIT_REAL").as_deref() == Ok("1")
}

fn workdir() -> PathBuf {
    let d = std::env::var_os("ANTHILL_REAL_WORKDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/opt/anthill-real"));
    std::fs::create_dir_all(&d).expect("workdir");
    d
}

fn compile(name: &str, source: &str) -> PathBuf {
    let dir = workdir();
    let src = dir.join(format!("{name}.c"));
    let bin = dir.join(name);
    std::fs::write(&src, source).unwrap();
    let status = Command::new("cc")
        .args(["-O1", "-pthread", "-o"])
        .arg(&bin)
        .arg(&src)
        .status()
        .expect("cc available");
    assert!(status.success(), "compiling {name}");
    bin
}

fn engine(bin: &PathBuf, trials: u64) -> Engine {
    let spec = TargetSpec {
        name: "t".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec![bin.display().to_string()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: None,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: trials,
            wall_secs: 300,
        },
    };
    let settings = HermitSettings {
        trial_timeout: Duration::from_secs(60),
        ..HermitSettings::default()
    };
    Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("t".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(settings)))
}

#[test]
fn finds_and_reproduces_a_real_threading_bug_by_seed() {
    if !enabled() {
        eprintln!("skipped: set ANTHILL_HERMIT_REAL=1 on x86-64 Linux with hermit installed");
        return;
    }
    let buggy = compile("double_init", BUGGY);
    let e = engine(&buggy, 60);
    let out = e.run(&RunRequest::new("t", 1), &mut |p| {
        eprintln!("  {}", p.message);
        true
    });
    let out = out.unwrap();
    assert_eq!(
        out.status,
        RunStatus::Failed,
        "no violating seed in 60 tries: {out:?}"
    );
    let f = out.failure.unwrap();
    eprintln!("found: {}", f.message);
    assert!(
        f.message.contains("reproduced 3/3"),
        "a Hermit failure must reproduce exactly: {}",
        f.message
    );
    let bundle = Bundle::decode(&f.bundle).unwrap();
    let again = e.replay(&bundle).unwrap();
    assert!(again.reproduced);
    assert!(again
        .steps
        .iter()
        .any(|s| s.label.contains("output matches the recording: yes")));
}

#[test]
fn the_fixed_program_passes_every_seed() {
    if !enabled() {
        return;
    }
    let fixed = compile("single_init", FIXED);
    let out = engine(&fixed, 60)
        .run(&RunRequest::new("t", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert_eq!(out.trials, 60);
}

#[test]
fn real_hermit_is_deterministic_for_both_programs() {
    if !enabled() {
        return;
    }
    for (name, src) in [("double_init", BUGGY), ("single_init", FIXED)] {
        let bin = compile(name, src);
        let r = engine(&bin, 1).doctor("t", 5, 0).unwrap();
        assert!(
            r.ok,
            "{name} must behave identically for the same seed: {r:?}"
        );
    }
}
