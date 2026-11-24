//! The driver and `Backend::run` reach the host through the same functions, shown three ways: one control restore through
//! `run_session` (built as the driver binary builds its session) and one through `DhyveBackend::run` start the VM alike and
//! read the same input files; for every way a restore can come out that the fake can produce, the class and reason `run()`
//! reports equal those in the driver's record; and the driver's code, parsed with its comments and strings removed, names
//! none of what belongs to the backend.
mod common;
use common::{quick, tiny_bounds, Rig};
use hairspring_core::point::encoding::sha256_hex;
use hairspring_dhyve::driver::{records_dir, run_session, steps_log, Plan};
use hairspring_dhyve::host::Timings;
use hairspring_dhyve::session::Session;
use hairspring_dhyve::vmname::{Stamp, VmName};
use hairspring_dhyve::{DhyveBackend, BACKEND_ID};
use hairspring_engine::backend::{Backend, RunRequest};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::report::{RunOutcome, RunStatus};
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_engine::trial_log::ReportedVerdict;
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;

const SEED: u64 = 1007;
const STAMP: &str = "20261201T0540Z";

fn timings() -> Timings {
    Timings {
        trial_cap: Duration::from_secs(1),
        resume_limit: Duration::from_millis(500),
        ..quick()
    }
}

fn host(rig: &Rig) -> hairspring_dhyve::host::HostConfig {
    let mut host = rig.host.clone();
    host.session_stamp = Some(Stamp::parse(STAMP).unwrap());
    host
}

/// The session as the driver binary builds it (the binary adds a pin check, which the fake's links cannot meet).
fn driver_session(rig: &Rig) -> Session {
    Session::new(host(rig), timings(), tiny_bounds())
        .with_one_prepare()
        .with_keep_all()
}

fn backend(rig: &Rig) -> DhyveBackend {
    DhyveBackend::with_settings(host(rig), timings(), tiny_bounds())
}

fn spec() -> TargetSpec {
    TargetSpec {
        name: "demo".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/true".into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: Some(1),
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
            pass_env: false,
            backend: Some(BACKEND_ID.into()),
        },
        budget: Budget::default(),
    }
}

fn run_once(rig: &Rig) -> RunOutcome {
    let budget = Budget {
        max_trials: 1,
        ..Budget::default()
    };
    backend(rig)
        .run(&spec(), &RunRequest::new("demo", SEED), budget, &mut |_| {
            true
        })
        .unwrap()
}

fn restore_record(rig: &Rig, session: &Session) -> Value {
    let path = records_dir(session).join(format!("vmvx_{STAMP}_XB_r02.json"));
    let _ = rig;
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The hash a record carries: the arguments with the VM name and the run directory masked, joined by line feeds.
fn masked_hash(rig: &Rig, words: &[String]) -> String {
    let root = rig.root.display().to_string();
    let run_dir = format!("{root}/st/runs/{STAMP}/r02");
    let name = format!("vmvx_{STAMP}_XB_r02");
    let text: Vec<String> = words
        .iter()
        .map(|w| {
            w.replace("ROOT", &root)
                .replace(&name, "NAME")
                .replace(&run_dir, "RUN")
        })
        .collect();
    sha256_hex(text.join("\n").as_bytes())
}

fn count(calls: &[String], needle: &str) -> usize {
    calls.iter().filter(|c| c.contains(needle)).count()
}

fn step_count(rig: &Rig, session: &Session, kind: &str) -> usize {
    let _ = rig;
    std::fs::read_to_string(steps_log(&records_dir(session)))
        .unwrap()
        .lines()
        .filter(|l| l.split('\t').nth(2) == Some(kind) && l.split('\t').nth(3) != Some("start"))
        .count()
}

#[test]
fn a_restore_through_run_session_and_one_through_run_start_the_vm_and_read_the_inputs_alike() {
    let rig = Rig::new("s4-paths");
    let session = driver_session(&rig);
    let summary = run_session(&session, &Plan::new(SEED, 1, 0));
    assert!(summary.clean(), "{}", summary.to_json());
    let record = restore_record(&rig, &session);
    let by_driver = (rig.start_words(), rig.inputs(), rig.calls());
    let driver_r5 = step_count(&rig, &session, "R5");
    let driver_r6 = step_count(&rig, &session, "R6");

    rig.reset();
    let outcome = run_once(&rig);
    assert_eq!(outcome.status, RunStatus::Passed, "{:?}", outcome.error);
    let by_run = (rig.start_words(), rig.inputs(), rig.calls());

    assert_eq!(
        by_driver.0, by_run.0,
        "the VM program was given the same arguments"
    );
    assert_eq!(by_driver.0.len(), 2, "a prepare and a restore");
    let (a, b) = (
        masked_hash(&rig, &by_driver.0[1]),
        masked_hash(&rig, &by_run.0[1]),
    );
    assert_eq!(a, b);
    assert_eq!(
        record["argv_masked_sha256"],
        a.as_str(),
        "the record's hash is that of the arguments the VM was given"
    );
    assert_eq!(
        by_driver.1, by_run.1,
        "the guest read the same six input files in both"
    );
    assert!(
        by_driver
            .1
            .iter()
            .all(|l| l.split(' ').nth(1) == Some("00000000000000000001")),
        "job 1: {:?}",
        by_driver.1
    );
    assert_eq!(
        count(&by_driver.2, "--destroy"),
        count(&by_run.2, "--destroy")
    );
    assert_eq!(count(&by_driver.2, "--checkpoint"), 1);
    assert_eq!(count(&by_run.2, "--checkpoint"), 1);
    assert_eq!(
        driver_r5,
        count(&by_driver.2, "--destroy"),
        "a row for each destroy"
    );
    assert_eq!(driver_r6, 1);
    let names: Vec<VmName> = by_run
        .0
        .iter()
        .map(|w| VmName::parse(w.last().unwrap()).expect("a name of the session's form"))
        .collect();
    assert_eq!((names[0].run_number(), names[1].run_number()), (1, 2));
    assert_eq!(record["verdict"], "pass");
    assert_eq!(outcome.trial_log[0].verdict, ReportedVerdict::Pass);
}

/// How a restore came out, in words both paths can give.
fn class_of_driver(record: &Value) -> (String, String) {
    (
        record["verdict"].as_str().unwrap().to_string(),
        record["reason"].as_str().unwrap().to_string(),
    )
}

fn class_of_run(outcome: &RunOutcome) -> (String, String) {
    match (outcome.status, outcome.trial_log.first()) {
        (RunStatus::Error, _) => ("infra".into(), outcome.error.clone().unwrap_or_default()),
        (RunStatus::Failed, Some(t)) => ("finding".into(), t.reason.clone()),
        (_, Some(t)) if t.verdict == ReportedVerdict::Inconclusive => {
            ("hang".into(), t.reason.clone())
        }
        (_, Some(t)) if t.verdict == ReportedVerdict::Unusable => {
            ("unusable".into(), t.reason.clone())
        }
        (_, Some(_)) => ("pass".into(), String::new()),
        (status, None) => (format!("{status:?}"), String::new()),
    }
}

#[test]
fn run_and_the_driver_give_the_same_class_and_reason_for_every_way_a_restore_can_come_out() {
    let modes: &[(&str, &str, &str)] = &[
        ("pass", "", "pass"),
        ("hang", "hang", "hang"),
        ("exit", "exit=3", "finding"),
        ("no_connect", "no_connect", "infra"),
        ("drop_after_resume", "drop_after_resume", "infra"),
        ("stale_stop", "stale_stop", "infra"),
        ("other_line", "other_line", "infra"),
        ("rip_mismatch", "rip_mismatch", "infra"),
        ("limit_log", "limit_log", "infra"),
        ("limit_open", "limit_open", "infra"),
        ("stop_then_exit", "stop_then_exit", "infra"),
        ("log_overrun", "log_overrun", "infra"),
        ("empty_uart", "empty_uart", "pass"),
        ("no_uart", "no_uart", "infra"),
        ("no_exit", "no_exit", "infra"),
        ("ready_differs", "ready_differs", "infra"),
        ("stopped_first", "stopped_first", "infra"),
    ];
    for (tag, mode, expected) in modes {
        let rig = Rig::new(&format!("s4-diff-{tag}"));
        rig.modes(&["", mode]);
        let session = driver_session(&rig);
        run_session(&session, &Plan::new(SEED, 1, 0));
        let (driver_class, driver_reason) = class_of_driver(&restore_record(&rig, &session));
        rig.reset();
        let outcome = run_once(&rig);
        let (run_class, run_reason) = class_of_run(&outcome);
        assert_eq!(&driver_class, expected, "{tag}: the driver");
        assert_eq!(driver_class, run_class, "{tag}: the class");
        assert_eq!(driver_reason, run_reason, "{tag}: the reason");
    }
}

#[test]
fn run_and_the_driver_agree_when_the_module_is_gone_or_changed_after_the_prepare() {
    for (tag, setting) in [("gone", "kld_after=2"), ("changed", "kld_line_after=2")] {
        let rig = Rig::new(&format!("s4-kld-{tag}"));
        rig.ctl(&[setting]);
        let session = driver_session(&rig);
        let summary = run_session(&session, &Plan::new(SEED, 1, 0));
        assert_eq!(summary.end, "END:VMM-CHANGED", "{tag}");
        let (driver_class, driver_reason) = class_of_driver(&restore_record(&rig, &session));
        rig.reset();
        let (run_class, run_reason) = class_of_run(&run_once(&rig));
        assert_eq!(
            (driver_class, driver_reason),
            (run_class, run_reason),
            "{tag}"
        );
    }
}

// ---- the driver's code, parsed -------------------------------------------------------------------------------------------

/// Source with comments and the insides of string and character literals removed, so only code is left to look at.
fn code_only(source: &str) -> String {
    let b: Vec<char> = source.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if b[i] == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if b[i] == '"' {
            i += 1;
            while i < b.len() && b[i] != '"' {
                i += if b[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            out.push_str("\"\"");
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

#[test]
fn the_drivers_code_calls_the_backends_prepare_restore_and_judge_and_names_nothing_of_the_host() {
    let driver = code_only(include_str!("../src/driver.rs"));
    let run = code_only(include_str!("../src/run.rs"));
    for (file, code) in [("run.rs", &run), ("driver.rs", &driver)] {
        for call in [".prepare(", ".restore_trial(", "judge_trial("] {
            assert!(code.contains(call), "{file} no longer calls {call}");
        }
    }
    for forbidden in [
        "director::Director",
        "Director::",
        "Spawner",
        "signal::",
        "startcheck",
        "vmargs",
        "crate::checkpoint",
        "runlog::",
        "logs::",
        "hostlock",
        "Inputs::write",
        ".write(&",
        "std::process",
        "std::os::unix::net",
        "Command::new",
        "UnixListener",
        "kill(",
        "spawn(",
    ] {
        assert!(!driver.contains(forbidden), "driver.rs names {forbidden}");
    }
}

#[test]
fn a_comment_does_not_count_as_a_call() {
    let code = code_only(
        "// session.prepare(\nfn f() { /* judge_trial( */ let s = \".restore_trial(\"; }",
    );
    for call in [".prepare(", ".restore_trial(", "judge_trial("] {
        assert!(!code.contains(call), "{call} survived");
    }
}
