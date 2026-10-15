//! Each trial's schedule is fingerprinted from Hermit's `--summary-json`, so a run can say how many distinct
//! interleavings it explored. A passing run prints the same line whatever the schedule, so output cannot tell.
use hairspring_core::bundle::Bundle;
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::erased::Registry;
use hairspring_engine::report::RunStatus;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Writes a summary whose deterministic fields depend on `seed % 3`; the wall-clock field differs every time.
const FAKE_HERMIT: &str = r#"#!/bin/sh
shift
seed=0
summary=""
while [ "$1" != "--" ]; do
  case "$1" in
    --sched-seed=*) seed=${1#--sched-seed=};;
    --summary-json=*) summary=${1#--summary-json=};;
  esac
  shift
done
shift
echo "$summary" >> "${FAKE_SUMMARY_LOG:-/dev/null}"
if [ -n "$summary" ]; then
  case "${FAKE_SUMMARY_MODE:-good}" in
    good) printf '{"sched_turns": %s, "syscalls": %s, "virttime_elapsed": 5, "realtime_elapsed": {"secs": %s, "nanos": 7}, "timeslice_stats": {"count": 1, "sum_ns": 2, "min_ns": 1, "max_ns": 3}, "per_thread_timeslice": []}' "$((seed % 3 + 10))" "$((100 + seed % 3))" "$$" > "$summary";;
    garbage) printf 'this is not json' > "$summary";;
    huge) head -c 3000000 /dev/zero | tr '\0' 'x' > "$summary";;
  esac
fi
echo "hermit: virtual-time epoch=fake" >&2
FAKE_SCHED_SEED=$seed exec "$@"
"#;

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hairspring-fp-{name}-{}-{nanos}",
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

struct Rig {
    engine: Engine,
    summaries: PathBuf,
}

fn rig(
    name: &str,
    command: &str,
    trials: u64,
    parallelism: u32,
    with_dir: bool,
    mode: &str,
) -> Rig {
    let dir = scratch(name);
    let summaries = dir.join("summaries");
    std::fs::create_dir_all(&summaries).unwrap();
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), command.into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: None,
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: trials,
            wall_secs: 60,
        },
    };
    let settings = HermitSettings {
        bin: fake(&dir),
        trial_timeout: Duration::from_secs(10),
        summary_dir: with_dir.then(|| summaries.clone()),
        extra_env: vec![("FAKE_SUMMARY_MODE".into(), mode.into())],
        ..HermitSettings::default()
    };
    let engine = Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(settings)));
    Rig { engine, summaries }
}

/// Files left anywhere under `dir` (the backend keeps its own private subdirectory there while it lives).
fn leftovers(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            if p.is_dir() {
                leftovers(&p)
            } else {
                1
            }
        })
        .sum()
}

#[test]
fn a_run_counts_distinct_schedules_and_total_work() {
    let r = rig("count", "echo fine", 6, 1, true, "good");
    let out = r
        .engine
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    // Seeds 1..6 give seed % 3 = 1,2,0,1,2,0: three distinct schedules, whatever the wall-clock field says.
    assert_eq!(out.distinct_schedules, 3, "{out:?}");
    assert_eq!(out.work, 2 * (101 + 102 + 100));
    assert_eq!(leftovers(&r.summaries), 0, "summary files are cleaned up");
}

#[test]
fn parallel_trials_write_separate_summaries_and_count_the_same() {
    let r = rig("parallel", "echo fine", 6, 3, true, "good");
    let out = r
        .engine
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(
        (out.distinct_schedules, out.work),
        (3, 2 * (101 + 102 + 100)),
        "{out:?}"
    );
    assert_eq!(leftovers(&r.summaries), 0);
}

#[test]
fn without_a_summary_directory_the_run_still_works_and_reports_unknown() {
    let r = rig("nodir", "echo fine", 3, 1, false, "good");
    let out = r
        .engine
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert_eq!((out.distinct_schedules, out.work), (0, 0));
}

#[test]
fn a_garbage_or_oversized_summary_is_ignored_not_trusted() {
    for mode in ["garbage", "huge"] {
        let r = rig(mode, "echo fine", 2, 1, true, mode);
        let out = r
            .engine
            .run(&RunRequest::new("svc", 1), &mut |_| true)
            .unwrap();
        assert_eq!(out.status, RunStatus::Passed, "{mode}: {out:?}");
        assert_eq!((out.distinct_schedules, out.work), (0, 0), "{mode}");
        assert_eq!(leftovers(&r.summaries), 0, "{mode}");
    }
}

fn recorded_bundle(r: &Rig) -> Bundle {
    let out = r
        .engine
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    Bundle::decode(&out.failure.expect("violation").bundle).unwrap()
}

const ALWAYS_FAILS: &str = "echo 'HAIRSPRING_VIOLATION: boom'; exit 1";

#[test]
fn bundles_record_the_schedule_fingerprint_and_replay_checks_it() {
    let r = rig("replay", ALWAYS_FAILS, 1, 1, true, "good");
    let bundle = recorded_bundle(&r);
    let fp = bundle
        .params
        .get("schedule_fingerprint")
        .cloned()
        .expect("fingerprint recorded");
    assert_eq!(fp.len(), 16, "{fp}");
    let same = r.engine.replay(&bundle).unwrap();
    assert!(same.reproduced);
    assert!(
        same.steps
            .iter()
            .any(|s| s.label.contains("schedule matches the recording: yes")),
        "{:?}",
        same.steps
    );
    let tampered = bundle
        .clone()
        .with_param("schedule_fingerprint", "0000000000000000");
    let diff = r.engine.replay(&tampered).unwrap();
    assert!(
        diff.steps
            .iter()
            .any(|s| s.label.contains("schedule matches the recording: no")),
        "{:?}",
        diff.steps
    );
}

#[test]
fn a_symlink_planted_where_the_private_directory_goes_is_not_followed() {
    use std::os::unix::fs::{symlink, MetadataExt};
    let r = rig("plant", "echo fine", 2, 1, true, "good");
    // `rig` already built the backend; build a second rig whose summary base holds a planted link at the exact path.
    let dir = scratch("plant2");
    let base = dir.join("summaries");
    std::fs::create_dir_all(&base).unwrap();
    let uid = std::fs::metadata(&base).unwrap().uid();
    let elsewhere = scratch("plant-target");
    symlink(
        &elsewhere,
        base.join(format!("hairspring-{uid}-{}", std::process::id())),
    )
    .unwrap();
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), "echo fine".into()],
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
            max_trials: 2,
            wall_secs: 60,
        },
    };
    let settings = HermitSettings {
        bin: fake(&dir),
        trial_timeout: Duration::from_secs(10),
        summary_dir: Some(base),
        ..HermitSettings::default()
    };
    let e = Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(settings)));
    let out = e.run(&RunRequest::new("svc", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert_eq!(
        (out.distinct_schedules, out.work),
        (0, 0),
        "fingerprints off, not redirected"
    );
    assert_eq!(
        leftovers(&elsewhere),
        0,
        "nothing was written through the link"
    );
    drop(r);
}
