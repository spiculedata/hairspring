//! Replaying through the service refuses a changed loader as its own error, and an explicit option allows it.
#![cfg(unix)]

mod common;

use common::setup;
use hairspring_engine::backend::ReplayOptions;
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::erased::Registry;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_engine::EngineError;
use hairspring_hermit::host_identity::HostSources;
use hairspring_hermit::{HermitBackend, HermitSettings};
use hairspring_service::ServiceError;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FAKE_HERMIT: &str =
    "#!/bin/sh\nshift\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n";

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hairspring-svcenv-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn engine(host: &Path) -> Engine {
    let bin = scratch("bin").join("fake-hermit");
    std::fs::write(&bin, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                "echo 'HAIRSPRING_VIOLATION: boom'; exit 1".into(),
            ],
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
        summary_dir: None,
        ..HermitSettings::default()
    };
    Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(
        HermitBackend::new(settings).with_host_sources(HostSources::rooted(host)),
    ))
}

#[test]
fn the_service_refuses_a_changed_loader_unless_told_otherwise() {
    let (svc, _, _) = setup();
    let host = scratch("host");
    put(&host, "etc/ld.so.cache", "cache-one");
    let e = engine(&host);
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    let bundle = out.failure.unwrap().bundle;
    assert!(svc.replay_bundle(&e, &bundle).unwrap().reproduced);

    put(&host, "etc/ld.so.cache", "cache-two");
    let err = svc.replay_bundle(&e, &bundle).unwrap_err();
    assert!(
        matches!(
            err,
            ServiceError::Engine(EngineError::EnvironmentMismatch(_))
        ),
        "{err:?}"
    );
    let allow = ReplayOptions {
        allow_environment_change: true,
    };
    let replay = svc.replay_bundle_with(&e, &bundle, allow).unwrap();
    assert!(replay.reproduced);
    assert!(replay.environment_changed);
    assert!(replay.warnings.iter().any(|w| w.contains("ld_so_cache")));
}
