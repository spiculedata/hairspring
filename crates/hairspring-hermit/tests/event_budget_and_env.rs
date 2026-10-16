//! The per-trial event budget as a setting, and the process tag the backend pins for the guest.
mod common;

use common::events::{rig, rig_with};
use common::props::ev;
use hairspring_engine::config::TargetKind;

/// One process declaring `ids` always-properties with long messages: about 850 bytes per id with its assert line.
fn big_report(ids: usize) -> String {
    let pad = "p".repeat(197);
    let thread = "t".repeat(64);
    let mut out = ev(
        r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"abc","signal_handlers":false}"#,
    );
    for i in 0..ids {
        out += &ev(&format!(
            r#"{{"v":1,"kind":"catalog","proc":"p1","id":"sqlite.property.number.{i:05}","type":"always","must_hit":true,"msg":"{pad}{pad}","loc":{{"file":"sqlite3.c","line":{},"func":"walCheckpoint"}}}}"#,
            i + 1
        ));
    }
    for i in 0..ids {
        out += &ev(&format!(
            r#"{{"v":1,"kind":"assert","proc":"p1","id":"sqlite.property.number.{i:05}","hits":3,"trues":2,"falses":1,"first_false":{{"thread":"{thread}","ord":{},"vals":[1,2,3,4]}},"first_true":{{"thread":"{thread}","ord":1,"vals":[1,2,3,4]}}}}"#,
            i + 1
        ));
    }
    out += &ev(&format!(
        r#"{{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":{ids},"evals":9,"dropped":0,"threads":0}}"#
    ));
    out
}

const MIB: usize = 1024 * 1024;

#[test]
fn by_default_a_report_of_thirteen_hundred_ids_over_stdout_arrives_whole() {
    let r = rig("budget-default", "cat $REPORT", false, vec![]);
    let text = big_report(1300);
    assert!(text.len() > MIB);
    std::fs::write(r.dir.join("report.events"), &text).unwrap();
    let t = r.run(1);
    let props = t.events.expect("events");
    assert_eq!(props.health.over_limit, 0, "{:?}", props.health);
    assert_eq!(props.properties.len(), 1300);
    assert!(!props.is_degraded());
}

#[test]
fn a_smaller_budget_can_be_set_and_is_enforced_by_the_capture_and_the_fold() {
    let r = rig_with("budget-small", "cat $REPORT", false, vec![], |s| {
        s.max_event_bytes = MIB;
    });
    std::fs::write(r.dir.join("report.events"), big_report(1300)).unwrap();
    let props = r.run(1).events.expect("events");
    assert!(props.health.over_limit > 0, "{:?}", props.health);
    assert!(props.health.event_bytes as usize <= MIB);
    assert!(props.is_degraded());
}

#[test]
fn the_events_file_obeys_the_same_budget() {
    let script = "cat $REPORT > \"$HAIRSPRING_EVENTS_FILE\"";
    let r = rig_with("budget-file", script, true, vec![], |s| {
        s.max_event_bytes = MIB;
    });
    std::fs::write(r.dir.join("report.events"), big_report(1300)).unwrap();
    let t = r.run(1);
    assert!(t.events.unwrap().health.over_limit > 0);
    assert!(
        t.events_error.is_some(),
        "a file past the budget is a guest fault"
    );
}

/// What the guest sees, one `NAME=value` per line, without the seed (which is meant to differ).
fn guest_env(r: &common::events::Rig, seed: u64) -> Vec<String> {
    r.run(seed)
        .stdout
        .lines()
        .filter(|l| !l.starts_with("HAIRSPRING_SEED="))
        .map(str::to_string)
        .collect()
}

fn hostile_rig(name: &str) -> common::events::Rig {
    let mut r = rig_with(name, "env | sort", false, vec![], |s| {
        s.extra_env.push(("HAIRSPRING_PROC".into(), "evil".into()));
        s.extra_env
            .push(("HAIRSPRING_PROC_PID".into(), "4242".into()));
    });
    if let TargetKind::Process { env, .. } = &mut r.spec.kind {
        env.insert("HAIRSPRING_PROC".into(), "configured".into());
        env.insert("HAIRSPRING_PROC_PID".into(), "7".into());
    }
    r
}

#[test]
fn the_guest_gets_a_fixed_root_process_tag_and_no_inherited_pid() {
    let r = hostile_rig("proc-env");
    let env = guest_env(&r, 1);
    let procs: Vec<_> = env
        .iter()
        .filter(|l| l.starts_with("HAIRSPRING_PROC"))
        .collect();
    assert_eq!(procs, vec!["HAIRSPRING_PROC=p1"], "{env:?}");
}

#[test]
fn the_guest_environment_is_identical_across_seeds_and_runs() {
    let r = hostile_rig("proc-env-same");
    let first = guest_env(&r, 1);
    assert_eq!(first, guest_env(&r, 2));
    assert_eq!(first, guest_env(&r, 987_654_321));
    assert_eq!(first, guest_env(&r, 1));
}

#[test]
fn a_clean_environment_gets_the_same_tag() {
    let r = rig("proc-env-clean", "env | sort", false, vec![]);
    let env = guest_env(&r, 3);
    assert!(env.contains(&"HAIRSPRING_PROC=p1".to_string()), "{env:?}");
    assert!(!env.iter().any(|l| l.starts_with("HAIRSPRING_PROC_PID")));
}
