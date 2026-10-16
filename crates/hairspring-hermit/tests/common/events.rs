//! A fake Hermit and a rig around it for tests of guest event capture.
use super::{scratch, write_fake};
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings, Trial};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Passes through to the guest. Logs what hermit itself was given. When asked to mount a `/tmp` (`--tmp=DIR`), it
/// stands in for Hermit's bind mount by pointing the guest's events variable at `DIR`. Marker files in `$EV_LOG`
/// make it misbehave: `plant` (a symlink where the events file goes), `lockparent` (makes `DIR`'s parent read-only).
pub const FAKE: &str = r#"#!/bin/sh
shift
seed=0
tmp=
args=
while [ "$1" != "--" ]; do
  case "$1" in
    --sched-seed=*) seed=${1#--sched-seed=};;
    --tmp=*) tmp=${1#--tmp=};;
  esac
  args="$args $1"
  shift
done
shift
if [ -n "$EV_LOG" ]; then
  echo "${HAIRSPRING_EVENTS_FILE-unset}" > "$EV_LOG/env-$seed"
  echo "$args" > "$EV_LOG/args-$seed"
fi
if [ -n "$tmp" ]; then
  export HAIRSPRING_EVENTS_FILE="$tmp/hairspring-events"
  if [ -e "$EV_LOG/plant" ]; then ln -s /etc/hosts "$tmp/hairspring-events"; fi
  if [ -e "$EV_LOG/lockparent" ]; then chmod 500 "$(dirname "$tmp")"; fi
fi
exec "$@"
"#;

/// One process's report: hello, one catalogued always-property that held, work, end.
pub const REPORT: &str = r#"HAIRSPRING_EVENT {"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"9f2c","signal_handlers":false}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"wal.ok","type":"always","must_hit":true,"msg":"holds"}
HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"wal.ok","hits":5,"trues":5,"falses":0}
HAIRSPRING_EVENT {"v":1,"kind":"work","proc":"p1","ops":300}
HAIRSPRING_EVENT {"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":5,"dropped":0,"threads":0}
"#;

pub struct Rig {
    pub dir: PathBuf,
    pub backend: HermitBackend,
    pub spec: TargetSpec,
}

pub fn rig(name: &str, script: &str, events_file: bool, hermit_args: Vec<String>) -> Rig {
    rig_with(name, script, events_file, hermit_args, |_| {})
}

/// `tweak` adjusts the settings (the fake hermit, a private summary directory and `EV_LOG` are already set).
pub fn rig_with(
    name: &str,
    script: &str,
    events_file: bool,
    hermit_args: Vec<String>,
    tweak: impl FnOnce(&mut HermitSettings),
) -> Rig {
    let dir = scratch(name);
    let report = dir.join("report.events");
    std::fs::write(&report, REPORT).unwrap();
    let script = script.replace("$REPORT", &report.display().to_string());
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), script],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args,
            trial_timeout_secs: Some(20),
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
    let scratch_base = dir.join("shm");
    std::fs::create_dir_all(&scratch_base).unwrap();
    let mut settings = HermitSettings {
        bin: write_fake(&dir, FAKE),
        trial_timeout: Duration::from_secs(20),
        summary_dir: Some(scratch_base),
        events_file,
        extra_env: vec![("EV_LOG".to_string(), dir.display().to_string())],
        ..HermitSettings::default()
    };
    tweak(&mut settings);
    Rig {
        dir,
        backend: HermitBackend::new(settings),
        spec,
    }
}

impl Rig {
    pub fn run(&self, seed: u64) -> Trial {
        self.backend.run_one(&self.spec, seed).unwrap()
    }

    pub fn set_timeout(&mut self, secs: u64) {
        if let TargetKind::Process {
            trial_timeout_secs, ..
        } = &mut self.spec.kind
        {
            *trial_timeout_secs = Some(secs);
        }
    }

    pub fn logged(&self, what: &str, seed: u64) -> String {
        std::fs::read_to_string(self.dir.join(format!("{what}-{seed}")))
            .unwrap()
            .trim()
            .to_string()
    }

    /// The host directory hermit was told to mount as the guest's `/tmp` for `seed`.
    pub fn tmp_dir(&self, seed: u64) -> PathBuf {
        let args = self.logged("args", seed);
        let arg = args
            .split_whitespace()
            .find_map(|a| a.strip_prefix("--tmp="))
            .expect("hermit was given --tmp");
        PathBuf::from(arg)
    }

    /// An engine over this rig's target and backend.
    pub fn engine(self) -> (Engine, PathBuf) {
        let engine = Engine::new(
            Arc::new(Registry::new()),
            Config {
                targets: BTreeMap::from([("svc".to_string(), self.spec.clone())]),
            },
        )
        .with_backend(Arc::new(self.backend));
        (engine, self.dir)
    }

    /// Entries left in the scratch base, below the private per-process directory.
    pub fn leftovers(&self) -> usize {
        leftovers_in(&self.dir.join("shm"))
    }
}

pub fn leftovers_in(base: &std::path::Path) -> usize {
    std::fs::read_dir(base)
        .unwrap()
        .flatten()
        .flat_map(|private| std::fs::read_dir(private.path()).unwrap().flatten())
        .count()
}
