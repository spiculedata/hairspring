//! Bundles record the execution environment, and replay reports how the host has changed since.
mod common;
use common::{scratch, write_fake, PASSTHROUGH};
use hairspring_core::bundle::Bundle;
use hairspring_core::environment::EnvironmentIdentity;
use hairspring_engine::backend::ReplayOptions;
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::erased::Registry;
use hairspring_engine::report::{ReplayOutcome, RunStatus};
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_engine::EngineError;
use hairspring_hermit::host_identity::HostSources;
use hairspring_hermit::{HermitBackend, HermitSettings};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const FAILS: &str = "echo 'HAIRSPRING_VIOLATION: boom'; exit 1";
const PASSES: &str = "echo fine";

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

fn engine(host: &Path, fake_body: &str, script: &str) -> Engine {
    engine_at(host, write_fake(&scratch("bin"), fake_body), script)
}

fn engine_at(host: &Path, bin: PathBuf, script: &str) -> Engine {
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
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
            max_trials: 1,
            wall_secs: 60,
        },
    };
    let settings = HermitSettings {
        bin,
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

fn record(host: &Path) -> Bundle {
    let out = engine(host, PASSTHROUGH, FAILS)
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    Bundle::decode(&out.failure.unwrap().bundle).unwrap()
}

fn has_warning(r: &ReplayOutcome, needle: &str) -> bool {
    r.warnings.iter().any(|w| w.contains(needle))
}

#[test]
fn a_new_bundle_is_v3_and_names_the_environment() {
    let host = fake_host("rec");
    let bundle = record(&host);
    let text = bundle.encode();
    assert!(text.starts_with("hairspring-bundle v3\n"), "{text}");
    let id = EnvironmentIdentity::from_bundle(&bundle).identity.unwrap();
    assert_eq!(id.get("host.kernel_release"), Some("6.8.0-45-generic"));
    assert_eq!(id.get("env.base_env"), Some("default"));
    assert!(id.get("host.loader.ld_so_cache.sha256").is_some());
    assert!(id.get("hermit.path.sha256").is_some());
}

#[test]
fn the_hermit_version_is_recorded_when_it_answers_and_skipped_when_it_hangs() {
    let host = fake_host("version");
    let answering = format!(
        "#!/bin/sh\n[ \"$1\" = \"--version\" ] && {{ echo 'hermit 9.9.9'; exit 0; }}\n{}",
        PASSTHROUGH.trim_start_matches("#!/bin/sh\n")
    );
    let run = |fake: &str| {
        let out = engine(&host, fake, FAILS)
            .run(&RunRequest::new("svc", 1), &mut |_| true)
            .unwrap();
        let bundle = Bundle::decode(&out.failure.unwrap().bundle).unwrap();
        EnvironmentIdentity::from_bundle(&bundle).identity.unwrap()
    };
    assert_eq!(run(&answering).get("hermit.version"), Some("hermit 9.9.9"));
    let hanging = format!(
        "#!/bin/sh\n[ \"$1\" = \"--version\" ] && sleep 30\n{}",
        PASSTHROUGH.trim_start_matches("#!/bin/sh\n")
    );
    let started = std::time::Instant::now();
    assert_eq!(run(&hanging).get("hermit.version"), None);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn replay_on_an_unchanged_host_has_no_environment_warning() {
    let host = fake_host("same");
    let bundle = record(&host);
    let r = engine(&host, PASSTHROUGH, FAILS).replay(&bundle).unwrap();
    assert!(r.reproduced, "{r:?}");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn a_different_kernel_release_is_reported_and_replay_still_runs() {
    let host = fake_host("kernel");
    let bundle = record(&host);
    put(&host, "proc/sys/kernel/osrelease", "6.9.1\n");
    let r = engine(&host, PASSTHROUGH, FAILS).replay(&bundle).unwrap();
    assert!(r.reproduced);
    assert!(
        has_warning(&r, "environment differs: host.kernel_release"),
        "{:?}",
        r.warnings
    );
}

#[test]
fn a_different_hermit_binary_is_reported() {
    let host = fake_host("hermit");
    let bundle = record(&host);
    let other = format!("{PASSTHROUGH}# a different build\n");
    let r = engine(&host, &other, FAILS).replay(&bundle).unwrap();
    assert!(has_warning(&r, "hermit.path.sha256"), "{:?}", r.warnings);
}

#[test]
fn a_rewritten_loader_cache_refuses_replay_as_infrastructure() {
    let host = fake_host("loader");
    let bundle = record(&host);
    put(&host, "etc/ld.so.cache", "cache-two");
    let err = engine(&host, PASSTHROUGH, FAILS)
        .replay(&bundle)
        .unwrap_err()
        .to_string();
    assert!(err.contains("environment differs"), "{err}");
    assert!(err.contains("host.loader.ld_so_cache.sha256"), "{err}");
    assert!(err.contains("allow-environment-change"), "{err}");
}

#[test]
fn an_explicit_override_replays_on_a_changed_loader_and_still_warns() {
    let host = fake_host("override");
    let bundle = record(&host);
    put(&host, "etc/ld.so.cache", "cache-two");
    let opts = ReplayOptions {
        allow_environment_change: true,
    };
    let r = engine(&host, PASSTHROUGH, FAILS)
        .replay_with(&bundle, opts)
        .unwrap();
    assert!(r.reproduced);
    assert!(
        has_warning(&r, "host.loader.ld_so_cache.sha256"),
        "{:?}",
        r.warnings
    );
}

#[test]
fn a_bundle_without_an_environment_replays_with_a_clear_note() {
    let host = fake_host("old");
    let recorded = record(&host);
    let old = Bundle {
        params: recorded
            .params
            .iter()
            .filter(|(k, _)| {
                !k.starts_with("host.")
                    && !k.starts_with("hermit.")
                    && !k.starts_with("env.")
                    && !k.starts_with("target.")
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        ..recorded
    };
    assert!(old.encode().starts_with("hairspring-bundle v2\n"));
    let r = engine(&host, PASSTHROUGH, FAILS).replay(&old).unwrap();
    assert!(r.reproduced);
    assert!(
        has_warning(&r, "no environment recorded"),
        "{:?}",
        r.warnings
    );
}

#[test]
fn a_miss_on_a_changed_host_is_reported_differently_from_a_miss_on_the_same_host() {
    let host = fake_host("miss");
    let bundle = record(&host);
    let unchanged = engine(&host, PASSTHROUGH, PASSES).replay(&bundle).unwrap();
    assert!(!unchanged.reproduced);
    assert!(unchanged.warnings.is_empty());
    assert!(unchanged
        .steps
        .iter()
        .all(|s| !s.label.contains("not evidence the bug is fixed")));
    put(&host, "proc/sys/kernel/osrelease", "6.9.1\n");
    let changed = engine(&host, PASSTHROUGH, PASSES).replay(&bundle).unwrap();
    assert!(!changed.reproduced);
    assert!(!unchanged.environment_changed);
    assert!(changed.environment_changed);
    assert!(has_warning(&changed, "environment differs"));
    assert!(
        changed
            .steps
            .iter()
            .any(|s| s.label.contains("not evidence the bug is fixed")),
        "{:?}",
        changed.steps
    );
}

fn changed_loader(host: &Path) {
    put(host, "etc/ld.so.cache", "cache-two");
}

#[test]
fn the_loader_refusal_is_its_own_error_kind() {
    let host = fake_host("kind");
    let bundle = record(&host);
    changed_loader(&host);
    let err = engine(&host, PASSTHROUGH, FAILS)
        .replay(&bundle)
        .unwrap_err();
    assert!(
        matches!(err, EngineError::EnvironmentMismatch(_)),
        "{err:?}"
    );
}

#[test]
fn a_hostile_bundle_with_thousands_of_params_gets_a_short_refusal() {
    let host = fake_host("hostile");
    let bundle = (0..5000).fold(record(&host), |b, i| {
        b.with_param(format!("host.field_{i}"), "x".repeat(200))
    });
    changed_loader(&host);
    let err = engine(&host, PASSTHROUGH, FAILS)
        .replay(&bundle)
        .unwrap_err();
    assert!(
        matches!(err, EngineError::EnvironmentMismatch(_)),
        "{err:?}"
    );
    assert!(err.to_string().len() < 1000, "{}", err.to_string().len());
}

#[test]
fn a_malformed_field_does_not_switch_off_the_loader_refusal() {
    let host = fake_host("malformed");
    let bundle = record(&host).with_param("host.container", "bad\nvalue");
    changed_loader(&host);
    let e = engine(&host, PASSTHROUGH, FAILS);
    assert!(matches!(
        e.replay(&bundle).unwrap_err(),
        EngineError::EnvironmentMismatch(_)
    ));
    let opts = ReplayOptions {
        allow_environment_change: true,
    };
    let r = e.replay_with(&bundle, opts).unwrap();
    assert!(has_warning(&r, "malformed"), "{:?}", r.warnings);
    assert!(has_warning(&r, "host.loader.ld_so_cache.sha256"));
}

#[test]
fn a_different_base_env_mode_is_reported_by_field_name() {
    let host = fake_host("baseenv");
    let bundle = record(&host).with_param("env.base_env", "host");
    let r = engine(&host, PASSTHROUGH, FAILS).replay(&bundle).unwrap();
    assert!(
        has_warning(&r, "env.base_env (recorded host, now default)"),
        "{:?}",
        r.warnings
    );
}

fn fake_with_prelude(prelude: &str) -> String {
    format!(
        "#!/bin/sh\n{prelude}\n{}",
        PASSTHROUGH.trim_start_matches("#!/bin/sh\n")
    )
}

fn version_of(bundle: &Bundle) -> Option<String> {
    EnvironmentIdentity::from_bundle(bundle)
        .identity
        .and_then(|id| id.get("hermit.version").map(str::to_string))
}

fn run_failing(e: &Engine) -> Bundle {
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    Bundle::decode(&out.failure.unwrap().bundle).unwrap()
}

#[test]
fn an_in_place_hermit_upgrade_does_not_record_the_old_version() {
    let host = fake_host("upgrade");
    let bin = write_fake(
        &scratch("upgrade-bin"),
        &fake_with_prelude("[ \"$1\" = \"--version\" ] && { echo 'hermit 1.0.0'; exit 0; }"),
    );
    let e = engine_at(&host, bin.clone(), FAILS);
    assert_eq!(
        version_of(&run_failing(&e)).as_deref(),
        Some("hermit 1.0.0")
    );
    write_fake(
        bin.parent().unwrap(),
        &fake_with_prelude("[ \"$1\" = \"--version\" ] && { echo 'hermit 2.0.0'; exit 0; }"),
    );
    assert_eq!(
        version_of(&run_failing(&e)).as_deref(),
        Some("hermit 2.0.0")
    );
}

#[test]
fn a_version_that_stops_answering_is_unavailable_not_changed() {
    let host = fake_host("unavailable");
    let flag = scratch("flag").join("hang");
    let fake = fake_with_prelude(&format!(
        "[ \"$1\" = \"--version\" ] && {{ [ -f {} ] && sleep 30; echo 'hermit 9.9.9'; exit 0; }}",
        flag.display()
    ));
    let bundle = run_failing(&engine(&host, &fake, FAILS));
    assert_eq!(version_of(&bundle).as_deref(), Some("hermit 9.9.9"));
    std::fs::write(&flag, "").unwrap();
    let r = engine(&host, &fake, FAILS).replay(&bundle).unwrap();
    assert!(
        has_warning(
            &r,
            "hermit.version (recorded hermit 9.9.9, now unavailable)"
        ),
        "{:?}",
        r.warnings
    );
}

#[test]
fn the_identity_is_taken_when_the_run_starts_not_after_the_reruns() {
    let host = fake_host("timing");
    let kernel = host.join("proc/sys/kernel/osrelease");
    let fake = fake_with_prelude(&format!(
        "[ \"$1\" = run ] && echo 6.9.9 > {}",
        kernel.display()
    ));
    let bundle = run_failing(&engine(&host, &fake, FAILS));
    let id = EnvironmentIdentity::from_bundle(&bundle).identity.unwrap();
    assert_eq!(id.get("host.kernel_release"), Some("6.8.0-45-generic"));
}
