//! Replaying across the change that made the engine name its variables for the guest under `--base-env=minimal`.
//!
//! The guest below fails when it cannot see `HAIRSPRING_SEED` (as under `minimal` before the change, or with
//! `pass_env = false`) and passes when it can. So a bundle recorded without the variables misses when replayed with
//! them, and the replay must say the environment changed rather than that the bug is fixed.
mod common;
use common::minimal_fake::FAKE;
use common::{scratch, write_fake};
use hairspring_core::bundle::Bundle;
use hairspring_core::environment::EnvironmentIdentity;
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::erased::Registry;
use hairspring_engine::report::{ReplayOutcome, RunStatus};
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::host_identity::HostSources;
use hairspring_hermit::{HermitBackend, HermitSettings};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const SCRIPT: &str =
    "if [ -n \"$HAIRSPRING_SEED\" ]; then echo fine; else echo 'HAIRSPRING_VIOLATION: no seed'; exit 1; fi";
const PASSED: &str = "env.guest.passed";
const NOT_EVIDENCE: &str = "not evidence the bug is fixed";
const MINIMAL: &[&str] = &["--base-env=minimal"];

fn put(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn fake_host(name: &str) -> PathBuf {
    let root = scratch(name);
    put(&root, "proc/sys/kernel/osrelease", "6.8.0-45-generic\n");
    put(
        &root,
        "proc/cpuinfo",
        "model name\t: Fake CPU\nflags\t\t: sse2\n",
    );
    put(&root, "etc/ld.so.cache", "cache-one");
    put(&root, "lib/x86_64-linux-gnu/libc.so.6", "libc-one");
    put(&root, "lib64/ld-linux-x86-64.so.2", "ld-one");
    root
}

fn engine(host: &Path, hermit_args: &[&str], pass_env: bool, script: &str) -> Engine {
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: hermit_args.iter().map(|a| a.to_string()).collect(),
            trial_timeout_secs: None,
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
            pass_env,
            backend: None,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: 1,
            wall_secs: 60,
        },
    };
    let settings = HermitSettings {
        bin: write_fake(&scratch("bin"), FAKE),
        trial_timeout: Duration::from_secs(10),
        summary_dir: None,
        ..HermitSettings::default()
    };
    let backend = HermitBackend::new(settings).with_host_sources(HostSources::rooted(host));
    Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(backend))
}

fn record(host: &Path, hermit_args: &[&str], pass_env: bool, script: &str) -> Bundle {
    let out = engine(host, hermit_args, pass_env, script)
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    Bundle::decode(&out.failure.unwrap().bundle).unwrap()
}

/// The bundle as an engine from before the field existed wrote it.
fn without_the_field(bundle: Bundle) -> Bundle {
    Bundle {
        params: bundle
            .params
            .iter()
            // A bundle from before the field also predates the list of recorded fields.
            .filter(|(k, _)| k.as_str() != PASSED && k.as_str() != "env.fields")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        ..bundle
    }
}

fn recorded_value(bundle: &Bundle) -> Option<String> {
    EnvironmentIdentity::from_bundle(bundle)
        .identity
        .and_then(|id| id.get(PASSED).map(str::to_string))
}

fn warned(r: &ReplayOutcome, needle: &str) -> bool {
    r.warnings.iter().any(|w| w.contains(needle))
}

fn says_not_evidence(r: &ReplayOutcome) -> bool {
    r.steps.iter().any(|s| s.label.contains(NOT_EVIDENCE))
}

#[test]
fn a_new_bundle_records_what_the_engine_passed() {
    let host = fake_host("rec");
    let minimal = record(&host, MINIMAL, false, SCRIPT);
    assert_eq!(recorded_value(&minimal).as_deref(), Some("none"));
    let host_default = record(&host, &[], true, "exit 1");
    assert_eq!(recorded_value(&host_default).as_deref(), Some("none"));
    let passing = record(&host, MINIMAL, true, "exit 1");
    assert_eq!(recorded_value(&passing).as_deref(), Some("hairspring-1"));
}

#[test]
fn an_old_minimal_bundle_replayed_with_the_new_default_reports_a_changed_environment() {
    let host = fake_host("old-minimal");
    let old = without_the_field(record(&host, MINIMAL, false, SCRIPT));
    let r = engine(&host, MINIMAL, true, SCRIPT).replay(&old).unwrap();
    assert!(!r.reproduced, "the guest now sees the seed and passes");
    assert!(r.environment_changed, "{r:?}");
    assert!(says_not_evidence(&r), "{:?}", r.steps);
    assert!(
        warned(&r, "env.guest.passed (recorded none, now hairspring-1)"),
        "{:?}",
        r.warnings
    );
    assert!(
        warned(
            &r,
            "set pass_env = false on the target to replay as recorded"
        ),
        "{:?}",
        r.warnings
    );
}

#[test]
fn an_old_minimal_bundle_replays_as_recorded_with_pass_env_off() {
    let host = fake_host("old-minimal-off");
    let old = without_the_field(record(&host, MINIMAL, false, SCRIPT));
    let r = engine(&host, MINIMAL, false, SCRIPT).replay(&old).unwrap();
    assert!(r.reproduced, "{r:?}");
    assert!(!r.environment_changed);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn an_old_host_bundle_still_compares_equal_under_either_setting() {
    let host = fake_host("old-host");
    let old = without_the_field(record(&host, &[], true, "exit 1"));
    for pass_env in [true, false] {
        let r = engine(&host, &[], pass_env, "exit 1").replay(&old).unwrap();
        assert!(r.reproduced, "{r:?}");
        assert!(!r.environment_changed);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }
}

#[test]
fn a_new_bundle_replayed_with_passing_off_reports_a_changed_environment() {
    let host = fake_host("new-off");
    let script = "if [ -e @FLAG@ ]; then echo fine; else exit 1; fi";
    let flag = scratch("flag").join("fixed");
    let script = script.replace("@FLAG@", &flag.display().to_string());
    let bundle = record(&host, MINIMAL, true, &script);
    assert_eq!(recorded_value(&bundle).as_deref(), Some("hairspring-1"));
    std::fs::write(&flag, "").unwrap();
    let r = engine(&host, MINIMAL, false, &script)
        .replay(&bundle)
        .unwrap();
    assert!(!r.reproduced);
    assert!(r.environment_changed, "{r:?}");
    assert!(says_not_evidence(&r), "{:?}", r.steps);
    assert!(
        warned(&r, "env.guest.passed=hairspring-1"),
        "{:?}",
        r.warnings
    );
}

#[test]
fn a_hit_across_the_change_still_reproduces_and_still_warns() {
    let host = fake_host("hit");
    let old = without_the_field(record(&host, MINIMAL, false, "exit 1"));
    let r = engine(&host, MINIMAL, true, "exit 1").replay(&old).unwrap();
    assert!(r.reproduced, "{r:?}");
    assert!(r.environment_changed);
    assert!(warned(&r, "env.guest.passed"), "{:?}", r.warnings);
}

#[test]
fn a_new_bundle_replayed_unchanged_has_no_warning() {
    let host = fake_host("new-same");
    let bundle = record(&host, MINIMAL, true, "exit 1");
    let r = engine(&host, MINIMAL, true, "exit 1")
        .replay(&bundle)
        .unwrap();
    assert!(r.reproduced);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}
