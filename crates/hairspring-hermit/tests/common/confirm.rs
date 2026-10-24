//! A fake `hermit` whose behaviour the test steers through small files next to it, and an engine built on it with a
//! failing target (`svc`) and a second target for the fixed build (`fixed`).
//!
//! Files in the fake's directory: `calls.log` (every invocation's arguments), `mode` (`plain` or `fpvary`, which gives
//! every call its own schedule summary), `skid_left` (how many base-margin calls report a skid overshoot),
//! `skid_double_fails` (the doubled margin overshoots too), `skid_base` (the margin that counts as the base one),
//! `skid_for` (only guests whose command contains this text overshoot), `verify` (`good`, `diverged` or `missing`),
//! `sleep` (seconds each call takes), `sleep_from` (the number of the first call that sleeps, counting every call the
//! fake has seen; `count` holds that number and a test may reset it).
use super::scratch;
use hairspring_core::bundle::Bundle;
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::erased::Registry;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const FAKE: &str = r#"#!/bin/sh
dir=$(dirname "$0")
[ "$1" = "--version" ] && { echo "fake-hermit 0.0"; exit 0; }
echo "$@" >> "$dir/calls.log"
n=$(cat "$dir/count" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$dir/count"
mode=$(cat "$dir/mode" 2>/dev/null || echo plain)
shift
seed=0
summary=""
vjson=""
skid=""
while [ "$1" != "--" ]; do
  case "$1" in
    --sched-seed=*) seed=${1#--sched-seed=};;
    --summary-json=*) summary=${1#--summary-json=};;
    --verify-json=*) vjson=${1#--verify-json=};;
    --skid-margin=*) skid=${1#--skid-margin=};;
  esac
  shift
done
shift
nap=$(cat "$dir/sleep" 2>/dev/null || echo 0)
from=$(cat "$dir/sleep_from" 2>/dev/null || echo 0)
[ "$nap" -gt 0 ] && [ "$n" -ge "$from" ] && sleep "$nap"
case "$*" in
  *INFRA_MARKER*) echo "HERMIT_INTERNAL_FAILURE class=fake" >&2; echo "Error: fake failure" >&2; exit 1;;
esac
only=$(cat "$dir/skid_for" 2>/dev/null)
if [ -n "$only" ]; then
  case "$*" in *"$only"*) ;; *) skid="";; esac
fi
if [ -n "$skid" ]; then
  left=$(cat "$dir/skid_left" 2>/dev/null || echo 0)
  base=$(cat "$dir/skid_base" 2>/dev/null || echo "$skid")
  if [ "$skid" = "$base" ] && [ "$left" -gt 0 ]; then
    echo $((left - 1)) > "$dir/skid_left"
    echo "SKID_OVERSHOOT" >&2
    exit 101
  fi
  if [ "$skid" != "$base" ] && [ -f "$dir/skid_double_fails" ]; then
    echo "SKID_OVERSHOOT" >&2
    exit 101
  fi
fi
if [ -n "$summary" ]; then
  turns=10
  [ "$mode" = fpvary ] && turns=$((10 + n))
  printf '{"sched_turns": %s, "syscalls": 100, "virttime_elapsed": 5, "timeslice_stats": {"count": 1}, "per_thread_timeslice": []}' "$turns" > "$summary"
fi
if [ -n "$vjson" ]; then
  case "$(cat "$dir/verify" 2>/dev/null || echo good)" in
    good) printf '{"verified": true, "bitwise_parity": true, "verdict": "matched"}' > "$vjson";;
    diverged) printf '{"verified": true, "bitwise_parity": false, "verdict": "diverged"}' > "$vjson";;
    missing) ;;
  esac
fi
echo "hermit: virtual-time epoch=fake" >&2
FAKE_SCHED_SEED=$seed exec "$@"
"#;

/// Violates when the seed is 3 modulo 5, so a run starting at 1 fails at seed 3.
pub const SEEDED_BUG: &str = "[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: balance went negative'; exit 1; }; echo fine";

#[derive(Clone)]
pub struct RigOptions {
    pub scheduler: SchedulerSpec,
    pub hermit_args: Vec<String>,
    pub control_hermit_args: Vec<String>,
    pub control_script: String,
    pub with_summaries: bool,
    pub extra_env: Vec<(String, String)>,
    /// The guest environment both targets carry (the target's own `env`).
    pub guest_env: Vec<(String, String)>,
    /// The failing target hangs on the first run after a test creates `hang_once` in the rig's directory, and times
    /// out after two seconds.
    pub hangs_once: bool,
}

impl Default for RigOptions {
    fn default() -> Self {
        RigOptions {
            scheduler: SchedulerSpec::default(),
            hermit_args: vec![],
            control_hermit_args: vec![],
            control_script: "echo fine".to_string(),
            with_summaries: true,
            extra_env: vec![],
            guest_env: vec![],
            hangs_once: false,
        }
    }
}

pub struct Rig {
    pub dir: PathBuf,
    pub engine: Engine,
}

fn process(
    name: &str,
    script: &str,
    hermit_args: Vec<String>,
    scheduler: SchedulerSpec,
    env: BTreeMap<String, String>,
    trial_timeout_secs: Option<u64>,
) -> TargetSpec {
    TargetSpec {
        name: name.into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
            env,
            cwd: None,
            hermit_args,
            trial_timeout_secs,
            scheduler,
            hang_is_violation: false,
            parallelism: 1,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: 20,
            wall_secs: 60,
        },
    }
}

pub fn rig(name: &str, o: RigOptions) -> Rig {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch(name);
    let summaries = dir.join("summaries");
    std::fs::create_dir_all(&summaries).unwrap();
    let bin = dir.join("fake-hermit");
    std::fs::write(&bin, FAKE).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let guest_env: BTreeMap<String, String> = o.guest_env.iter().cloned().collect();
    let hang = format!(
        "[ -f {d}/hang_once ] && {{ rm {d}/hang_once; sleep 30; }}; {SEEDED_BUG}",
        d = dir.display()
    );
    let svc = process(
        "svc",
        if o.hangs_once { &hang } else { SEEDED_BUG },
        o.hermit_args.clone(),
        o.scheduler.clone(),
        guest_env.clone(),
        o.hangs_once.then_some(2),
    );
    let fixed = process(
        "fixed",
        &o.control_script,
        o.control_hermit_args,
        o.scheduler,
        guest_env,
        None,
    );
    let settings = HermitSettings {
        bin,
        trial_timeout: Duration::from_secs(10),
        summary_dir: o.with_summaries.then_some(summaries),
        extra_env: o.extra_env,
        ..HermitSettings::default()
    };
    let engine = Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), svc), ("fixed".to_string(), fixed)]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(settings)));
    Rig { dir, engine }
}

impl Rig {
    pub fn set(&self, file: &str, value: &str) {
        std::fs::write(self.dir.join(file), value).unwrap();
    }

    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    pub fn clear_calls(&self) {
        let _ = std::fs::remove_file(self.dir.join("calls.log"));
    }

    /// The bundle of the failure a search finds from seed 1.
    pub fn bundle(&self) -> Bundle {
        let out = self
            .engine
            .run(&RunRequest::new("svc", 1), &mut |_| true)
            .unwrap();
        let failure = out.failure.expect("the search finds the seeded bug");
        let bundle = Bundle::decode(&failure.bundle).unwrap();
        self.clear_calls();
        bundle
    }
}
