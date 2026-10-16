//! Shared helpers for tests that drive the Hermit backend through a fake `hermit` script.
#![allow(dead_code)]
pub mod events;
pub mod multi;
pub mod props;
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Runs the guest after the `--`, exporting the seed; ignores every other flag.
pub const PASSTHROUGH: &str = r#"#!/bin/sh
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

pub fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hairspring-rf-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn write_fake(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("fake-hermit");
    std::fs::write(&p, body).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[derive(Clone)]
pub struct Opts {
    pub trials: u64,
    pub parallelism: u32,
    pub scheduler: SchedulerSpec,
    pub hermit_args: Vec<String>,
    pub timeout_secs: Option<u64>,
    pub hang_is_violation: bool,
    pub wall_secs: u64,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            trials: 1,
            parallelism: 1,
            scheduler: SchedulerSpec::default(),
            hermit_args: vec![],
            timeout_secs: None,
            hang_is_violation: false,
            wall_secs: 60,
        }
    }
}

pub fn engine(
    dir: &Path,
    fake_body: &str,
    script: &str,
    o: Opts,
    extra_env: &[(&str, &str)],
) -> Engine {
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: o.hermit_args,
            trial_timeout_secs: o.timeout_secs,
            scheduler: o.scheduler,
            hang_is_violation: o.hang_is_violation,
            parallelism: o.parallelism,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: o.trials,
            wall_secs: o.wall_secs,
        },
    };
    let settings = HermitSettings {
        bin: write_fake(dir, fake_body),
        trial_timeout: Duration::from_secs(10),
        summary_dir: None,
        extra_env: extra_env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
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
