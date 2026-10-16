//! Guest event lines are captured apart from the capped output, from stdout, stderr and the events file.
mod common;

use common::{scratch, write_fake};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings, Trial};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Passes through to the guest. Logs what hermit itself was given. When asked to mount a `/tmp` (`--tmp=DIR`), it
/// stands in for Hermit's bind mount by pointing the guest's events variable at `DIR`.
const FAKE: &str = r#"#!/bin/sh
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
fi
exec "$@"
"#;

/// One process's report: hello, one catalogued always-property that held, work, end.
const REPORT: &str = r#"HAIRSPRING_EVENT {"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"9f2c","signal_handlers":false}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"wal.ok","type":"always","must_hit":true,"msg":"holds"}
HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"wal.ok","hits":5,"trues":5,"falses":0}
HAIRSPRING_EVENT {"v":1,"kind":"work","proc":"p1","ops":300}
HAIRSPRING_EVENT {"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":5,"dropped":0,"threads":0}
"#;

struct Rig {
    dir: PathBuf,
    backend: HermitBackend,
    spec: TargetSpec,
}

fn rig(name: &str, script: &str, events_file: bool, hermit_args: Vec<String>) -> Rig {
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
    let settings = HermitSettings {
        bin: write_fake(&dir, FAKE),
        trial_timeout: Duration::from_secs(20),
        summary_dir: Some(scratch_base),
        events_file,
        extra_env: vec![("EV_LOG".to_string(), dir.display().to_string())],
        ..HermitSettings::default()
    };
    Rig {
        dir,
        backend: HermitBackend::new(settings),
        spec,
    }
}

impl Rig {
    fn run(&self, seed: u64) -> Trial {
        self.backend.run_one(&self.spec, seed).unwrap()
    }

    fn logged(&self, what: &str, seed: u64) -> String {
        std::fs::read_to_string(self.dir.join(format!("{what}-{seed}")))
            .unwrap()
            .trim()
            .to_string()
    }
}

fn leftovers(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("shm"))
        .unwrap()
        .flatten()
        .flat_map(|private| std::fs::read_dir(private.path()).unwrap().flatten())
        .count()
}

#[test]
fn a_report_printed_after_a_megabyte_of_chatter_survives() {
    let r = rig(
        "chatter",
        "head -c 2000000 /dev/zero | tr '\\0' x; echo; cat $REPORT",
        false,
        vec![],
    );
    let t = r.run(1);
    assert_eq!(
        t.stdout.len(),
        1024 * 1024,
        "the ordinary tail is still capped at 1 MiB"
    );
    assert!(t.stdout.bytes().all(|b| b == b'x'));
    let props = t.events.expect("the report was captured");
    assert_eq!(props.health.accepted, 5);
    assert!(!props.is_degraded());
    assert!(props.procs[0].end.is_some());
    assert_eq!(props.total_work(), 300);
}

#[test]
fn event_lines_do_not_appear_in_the_captured_output() {
    let r = rig(
        "clean",
        "echo before; cat $REPORT; echo after",
        false,
        vec![],
    );
    let t = r.run(1);
    assert_eq!(t.stdout, "before\nafter\n");
    assert!(t.events.is_some());
}

#[test]
fn stdout_and_stderr_events_are_merged() {
    let r = rig(
        "both",
        "cat $REPORT; sed 's/\"p1\"/\"p2\"/' $REPORT >&2",
        false,
        vec![],
    );
    let props = r.run(1).events.expect("events");
    let tags: Vec<_> = props.procs.iter().map(|p| p.tag.as_str()).collect();
    assert_eq!(tags, vec!["p1", "p2"]);
    assert!(props.procs.iter().all(|p| p.end.is_some()));
}

#[test]
fn malformed_lines_are_counted_and_the_rest_still_folds() {
    let r = rig(
        "malformed",
        "echo 'HAIRSPRING_EVENT {not json'; echo 'HAIRSPRING_EVENT{\"v\":1}'; cat $REPORT; exit 0",
        false,
        vec![],
    );
    let t = r.run(1);
    assert_eq!(t.exit_code, Some(0));
    let props = t.events.expect("events");
    assert_eq!(props.health.malformed, 2);
    assert_eq!(props.health.accepted, 5);
    assert!(props.is_degraded());
}

#[test]
fn event_lines_over_the_byte_budget_are_counted_not_stored() {
    // 400 lines of about 3 KiB: well over 1 MiB, on two streams.
    let script = "pad=$(head -c 3000 /dev/zero | tr '\\0' a); \
        i=0; while [ $i -lt 200 ]; do \
        echo \"HAIRSPRING_EVENT {\\\"v\\\":1,\\\"kind\\\":\\\"work\\\",\\\"proc\\\":\\\"p1\\\",\\\"ops\\\":1,\\\"x\\\":\\\"$pad\\\"}\"; \
        echo \"HAIRSPRING_EVENT {\\\"v\\\":1,\\\"kind\\\":\\\"work\\\",\\\"proc\\\":\\\"p1\\\",\\\"ops\\\":1,\\\"x\\\":\\\"$pad\\\"}\" >&2; \
        i=$((i+1)); done; cat $REPORT";
    let r = rig("overlimit", script, false, vec![]);
    let t = r.run(1);
    assert_eq!(t.exit_code, Some(0));
    let props = t.events.expect("events");
    assert!(props.health.over_limit > 0, "{:?}", props.health);
    assert!(props.health.event_bytes <= 1024 * 1024);
    assert!(props.is_degraded());
}

#[test]
fn a_line_over_4096_bytes_is_counted_as_too_long() {
    let r = rig(
        "toolong",
        "printf 'HAIRSPRING_EVENT '; head -c 100000 /dev/zero | tr '\\0' a; echo; cat $REPORT",
        false,
        vec![],
    );
    let props = r.run(1).events.expect("events");
    assert_eq!(props.health.too_long, 1);
    assert_eq!(props.health.accepted, 5);
}

#[test]
fn a_guest_without_events_is_unchanged() {
    let r = rig(
        "none",
        "echo hi; echo HAIRSPRING_EVENTS_FILE=x; echo oops >&2; exit 3",
        false,
        vec![],
    );
    let t = r.run(1);
    assert!(t.events.is_none());
    assert!(t.events_error.is_none());
    assert_eq!(t.stdout, "hi\nHAIRSPRING_EVENTS_FILE=x\n");
    assert_eq!(t.stderr, "oops\n");
    assert_eq!(t.exit_code, Some(3));
}

#[test]
fn without_the_setting_the_guest_gets_no_events_variable_and_no_tmp_mount() {
    let r = rig("off", "echo ${HAIRSPRING_EVENTS_FILE-unset}", false, vec![]);
    let t = r.run(4);
    assert_eq!(t.stdout, "unset\n");
    assert_eq!(r.logged("env", 4), "unset");
    assert!(!r.logged("args", 4).contains("--tmp"));
}

#[test]
fn the_events_file_is_read_and_folded() {
    let r = rig(
        "file",
        "cat $REPORT > \"$HAIRSPRING_EVENTS_FILE\"; echo done",
        true,
        vec![],
    );
    let t = r.run(2);
    assert_eq!(t.stdout, "done\n");
    assert!(t.events_error.is_none(), "{:?}", t.events_error);
    let props = t.events.expect("events from the file");
    assert_eq!(props.health.accepted, 5);
    assert!(props.procs[0].end.is_some());
    assert_eq!(leftovers(&r.dir), 0, "the per-trial directory is removed");
}

#[test]
fn file_and_stream_events_are_merged() {
    let r = rig(
        "filestream",
        "cat $REPORT > \"$HAIRSPRING_EVENTS_FILE\"; sed 's/\"p1\"/\"p2\"/' $REPORT",
        true,
        vec![],
    );
    let props = r.run(2).events.expect("events");
    assert_eq!(props.procs.len(), 2);
}

#[test]
fn the_guest_sees_the_same_events_path_for_every_seed() {
    let r = rig("fixed", "echo $HAIRSPRING_EVENTS_FILE", true, vec![]);
    let _ = (r.run(1), r.run(2), r.run(987_654_321));
    let seen: Vec<String> = [1, 2, 987_654_321]
        .iter()
        .map(|s| r.logged("env", *s))
        .collect();
    assert_eq!(seen[0], hairspring_hermit::GUEST_EVENTS_FILE);
    assert!(seen.iter().all(|s| *s == seen[0]), "{seen:?}");
    // The per-trial part is on hermit's own command line only, and differs between trials.
    let (a, b) = (r.logged("args", 1), r.logged("args", 2));
    assert!(a.contains("--tmp=") && b.contains("--tmp="));
    assert_ne!(a, b);
}

#[test]
fn a_file_over_the_budget_is_cut_and_counted() {
    let r = rig(
        "bigfile",
        "head -c 1500000 /dev/zero | tr '\\0' a > \"$HAIRSPRING_EVENTS_FILE\"; cat $REPORT",
        true,
        vec![],
    );
    let props = r.run(1).events.expect("events");
    assert!(props.health.over_limit >= 1);
    assert!(props.is_degraded());
}

#[test]
fn a_planted_symlink_is_refused_and_reported() {
    let r = rig("symlink", "cat $REPORT", true, vec![]);
    std::fs::write(r.dir.join("plant"), "").unwrap();
    let backend_trial = r.run(1);
    // The stream events still arrive; the unreadable file is reported, not trusted.
    assert!(backend_trial.events.is_some());
    assert!(backend_trial.events_error.is_some());
}

#[test]
fn a_target_that_sets_its_own_tmp_gets_no_events_file_and_a_reason() {
    let r = rig(
        "ownTmp",
        "echo ${HAIRSPRING_EVENTS_FILE-unset}",
        true,
        vec!["--tmp=/somewhere".to_string()],
    );
    let t = r.run(1);
    // What hermit itself was handed (the stub's own `--tmp` handling would otherwise mask it).
    assert_eq!(r.logged("env", 1), "unset");
    assert!(t.events_error.unwrap().contains("--tmp"));
}

#[test]
fn without_a_scratch_directory_the_reason_is_reported() {
    let dir = scratch("noscratch");
    let spec = rig("noscratch2", "echo hi", true, vec![]).spec;
    let backend = HermitBackend::new(HermitSettings {
        bin: write_fake(&dir, FAKE),
        summary_dir: None,
        events_file: true,
        ..HermitSettings::default()
    });
    let t = backend.run_one(&spec, 1).unwrap();
    assert_eq!(t.stdout, "hi\n");
    assert!(t.events_error.unwrap().contains("scratch"));
}
