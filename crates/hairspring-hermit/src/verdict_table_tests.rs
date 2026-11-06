//! A table of raw trial outcomes and the verdict each one gets. It pins what the backend decides today, through the
//! backend's own entry point (`HermitBackend::verdict`), so that moving the ladder around cannot change a verdict.
use crate::events::EventsFault;
use crate::exec::{ProcessSpec, Trial};
use crate::line_scan::Findings;
use crate::trace::{skid_overshoot, Verdict};
use crate::{HermitBackend, HermitSettings};
use hairspring_props::TrialEvents;
use std::collections::BTreeMap;
use std::time::Duration;

const TIMEOUT_SECS: u64 = 7;
const DEADLOCK_LINE: &str =
    "Deadlock detected: thread(s) waiting on futex, but no runnable threads left.";

const HELLO: &str = r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#;

fn event(json: &str) -> String {
    format!("HAIRSPRING_EVENT {json}\n")
}

/// A guest's report: one `always` property that was false once, and optionally a clean end.
fn false_report(end: bool) -> String {
    let mut text = event(HELLO);
    text += &event(
        r#"{"v":1,"kind":"catalog","proc":"p1","id":"x.id","type":"always","must_hit":true}"#,
    );
    text += &event(
        r#"{"v":1,"kind":"assert","proc":"p1","id":"x.id","hits":2,"trues":1,"falses":1,"first_false":{"thread":"t","ord":4,"vals":[0,0,0,0]},"first_true":null}"#,
    );
    if end {
        text += &event(
            r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":2,"dropped":0,"threads":0}"#,
        );
    }
    text
}

/// A guest's report with every property true, and optionally a clean end.
fn true_report(end: bool) -> String {
    let mut text = event(HELLO);
    text += &event(
        r#"{"v":1,"kind":"catalog","proc":"p1","id":"y.id","type":"always","must_hit":true}"#,
    );
    text += &event(
        r#"{"v":1,"kind":"assert","proc":"p1","id":"y.id","hits":2,"trues":2,"falses":0,"first_false":null,"first_true":null}"#,
    );
    if end {
        text += &event(
            r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":2,"dropped":0,"threads":0}"#,
        );
    }
    text
}

fn exited(code: i32) -> Trial {
    Trial {
        exit_code: Some(code),
        ..Trial::default()
    }
}

fn killed(signal: i32) -> Trial {
    Trial {
        signal: Some(signal),
        ..Trial::default()
    }
}

fn stdout(code: i32, text: &str) -> Trial {
    Trial {
        stdout: text.to_string(),
        ..exited(code)
    }
}

fn stderr(code: i32, text: &str) -> Trial {
    Trial {
        stderr: text.to_string(),
        ..exited(code)
    }
}

fn timed_out(base: Trial) -> Trial {
    Trial {
        timed_out: true,
        ..base
    }
}

fn with_events(text: &str, base: Trial) -> Trial {
    Trial {
        events: Some(TrialEvents::from_output(text.as_bytes()).fold()),
        events_digest: Some(1),
        ..base
    }
}

fn with_fault(fault: EventsFault, base: Trial) -> Trial {
    Trial {
        events_error: Some(fault),
        ..base
    }
}

fn show(v: &Verdict) -> String {
    match v {
        Verdict::Pass => "pass".to_string(),
        Verdict::Violation(m) => format!("violation: {m}"),
        Verdict::Hang(m) => format!("hang: {m}"),
        Verdict::Deadlock(m) => format!("deadlock: {m}"),
        Verdict::Infra(m) => format!("infra: {m}"),
        Verdict::Inconclusive(m) => format!("inconclusive: {m}"),
    }
}

fn judge(t: &Trial, hang_is_violation: bool) -> String {
    let command = vec!["./guest".to_string()];
    let env = BTreeMap::new();
    let scheduler = Default::default();
    let spec = ProcessSpec {
        target: "svc",
        command: &command,
        env: &env,
        cwd: None,
        hermit_args: &[],
        trial_timeout_secs: None,
        scheduler: &scheduler,
        hang_is_violation,
        parallelism: 1,
        pass_env: true,
    };
    let backend = HermitBackend::new(HermitSettings::default());
    show(&backend.verdict(&spec, t, Duration::from_secs(TIMEOUT_SECS)))
}

/// Each row: a name, the raw outcome, the target's `hang_is_violation`, and the verdict it gets.
fn assert_table(rows: Vec<(&str, Trial, bool, String)>) {
    let wrong: Vec<String> = rows
        .iter()
        .filter_map(|(name, trial, hang_is_violation, want)| {
            let got = judge(trial, *hang_is_violation);
            (&got != want).then(|| {
                format!(
                    "{name} (hang_is_violation={hang_is_violation}): got `{got}`, want `{want}`"
                )
            })
        })
        .collect();
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

fn hang() -> String {
    format!("hang: timed out after {TIMEOUT_SECS}s (possible hang or deadlock)")
}

fn hang_as_violation() -> String {
    format!("violation: timed out after {TIMEOUT_SECS}s (possible hang or deadlock)")
}

#[test]
fn exit_status_and_signals() {
    assert_table(vec![
        ("clean exit", exited(0), false, "pass".into()),
        ("exit 3", exited(3), false, "violation: exit code 3".into()),
        ("exit 1", exited(1), true, "violation: exit code 1".into()),
        (
            "exit 255",
            exited(255),
            false,
            "violation: exit code 255".into(),
        ),
        (
            "killed by 9",
            killed(9),
            false,
            "violation: terminated by signal 9".into(),
        ),
        (
            "killed by 11",
            killed(11),
            true,
            "violation: terminated by signal 11".into(),
        ),
        (
            "no status",
            Trial::default(),
            false,
            "violation: terminated abnormally".into(),
        ),
        (
            "exit 0 with a stray signal",
            Trial {
                signal: Some(11),
                ..exited(0)
            },
            false,
            "pass".into(),
        ),
        (
            "exit code wins over signal",
            Trial {
                signal: Some(9),
                ..exited(2)
            },
            false,
            "violation: exit code 2".into(),
        ),
    ]);
}

#[test]
fn timeouts_are_never_a_pass() {
    assert_table(vec![
        ("timeout", timed_out(Trial::default()), false, hang()),
        (
            "timeout, hang is violation",
            timed_out(Trial::default()),
            true,
            hang_as_violation(),
        ),
        (
            "timeout with a clean exit status",
            timed_out(exited(0)),
            false,
            hang(),
        ),
        (
            "timeout with a clean exit status, hang is violation",
            timed_out(exited(0)),
            true,
            hang_as_violation(),
        ),
        ("timeout, killed by 9", timed_out(killed(9)), false, hang()),
        ("timeout with exit 1", timed_out(exited(1)), false, hang()),
        (
            "timeout outranks a violation line",
            timed_out(stdout(1, "HAIRSPRING_VIOLATION: boom\n")),
            false,
            hang(),
        ),
        (
            "timeout outranks a property failure",
            timed_out(with_events(&false_report(true), exited(0))),
            false,
            hang(),
        ),
        (
            "timeout outranks a deadlock report",
            timed_out(stderr(1, &format!("{DEADLOCK_LINE}\n"))),
            false,
            hang(),
        ),
        (
            "timeout outranks a host fault",
            timed_out(with_fault(
                EventsFault::Infra("disk".into()),
                Trial::default(),
            )),
            false,
            hang(),
        ),
        (
            "internal failure outranks a timeout",
            timed_out(stderr(1, "HERMIT_INTERNAL_FAILURE\nError: no\n")),
            false,
            "infra: hermit failed to run the program: Error: no".into(),
        ),
        (
            "config fault outranks a timeout",
            timed_out(with_fault(
                EventsFault::Config("bad".into()),
                Trial::default(),
            )),
            true,
            "infra: bad".into(),
        ),
    ]);
}

#[test]
fn violation_markers() {
    assert_table(vec![
        (
            "stdout marker",
            stdout(1, "x\nHAIRSPRING_VIOLATION: boom\n"),
            false,
            "violation: boom".into(),
        ),
        (
            "stdout marker with a clean exit",
            stdout(0, "HAIRSPRING_VIOLATION: late\n"),
            false,
            "violation: late".into(),
        ),
        (
            "stderr marker",
            stderr(0, "HAIRSPRING_VIOLATION: err\n"),
            false,
            "violation: err".into(),
        ),
        (
            "legacy marker",
            stdout(0, "ANTHILL_VIOLATION: old\n"),
            false,
            "violation: old".into(),
        ),
        (
            "marker without a reason",
            stdout(1, "HAIRSPRING_VIOLATION\n"),
            false,
            "violation: property violated".into(),
        ),
        (
            "marker with only a colon",
            stdout(1, "HAIRSPRING_VIOLATION:  \n"),
            false,
            "violation: property violated".into(),
        ),
        (
            "indented marker",
            stdout(0, "   HAIRSPRING_VIOLATION: pad\n"),
            false,
            "violation: pad".into(),
        ),
        (
            "crlf marker",
            stdout(0, "HAIRSPRING_VIOLATION: crlf\r\n"),
            false,
            "violation: crlf".into(),
        ),
        (
            "stdout marker wins over a stderr marker",
            Trial {
                stderr: "HAIRSPRING_VIOLATION: second\n".into(),
                ..stdout(0, "HAIRSPRING_VIOLATION: first\n")
            },
            false,
            "violation: first".into(),
        ),
        (
            "the first marker line wins",
            stdout(0, "ANTHILL_VIOLATION: one\nHAIRSPRING_VIOLATION: two\n"),
            false,
            "violation: one".into(),
        ),
        (
            "marker text inside a line",
            stdout(0, "see HAIRSPRING_VIOLATION: no\n"),
            false,
            "pass".into(),
        ),
        (
            "marker text inside a line, exit 1",
            stdout(1, "xHAIRSPRING_VIOLATION: no\n"),
            false,
            "violation: exit code 1".into(),
        ),
        (
            "a marker in the omitted middle of a long stream still counts",
            Trial {
                stdout_scan: Some(Findings {
                    violation_line: Some("HAIRSPRING_VIOLATION: middle".into()),
                    ..Findings::default()
                }),
                ..stdout(0, "only the tail is kept\n")
            },
            false,
            "violation: middle".into(),
        ),
        (
            "the stream's own scan decides, not the kept text",
            Trial {
                stdout_scan: Some(Findings::default()),
                ..stdout(0, "HAIRSPRING_VIOLATION: kept text only\n")
            },
            false,
            "pass".into(),
        ),
        (
            "marker outranks a deadlock report",
            Trial {
                stderr: format!("{DEADLOCK_LINE}\n"),
                ..stdout(1, "HAIRSPRING_VIOLATION: marked\n")
            },
            false,
            "violation: marked".into(),
        ),
    ]);
}

#[test]
fn deadlock_reports() {
    let deadlock = format!("deadlock: deadlock: hermit reported `{DEADLOCK_LINE}` (exit code 1)");
    assert_table(vec![
        (
            "report with exit 1",
            stderr(1, &format!("hermit: start\n{DEADLOCK_LINE}\n")),
            false,
            deadlock.clone(),
        ),
        (
            "report, hang is violation",
            stderr(1, &format!("{DEADLOCK_LINE}\n")),
            true,
            deadlock.clone(),
        ),
        (
            "indented report",
            stderr(1, &format!("  {DEADLOCK_LINE}\r\n")),
            false,
            deadlock.clone(),
        ),
        (
            "report with exit 0",
            stderr(0, &format!("{DEADLOCK_LINE}\n")),
            false,
            "pass".into(),
        ),
        (
            "report with exit 2",
            stderr(2, &format!("{DEADLOCK_LINE}\n")),
            false,
            format!("deadlock: deadlock: hermit reported `{DEADLOCK_LINE}` (exit code 2)"),
        ),
        (
            "report with only a signal is a signal death",
            Trial {
                stderr: format!("{DEADLOCK_LINE}\n"),
                ..killed(6)
            },
            false,
            "violation: terminated by signal 6".into(),
        ),
        (
            "report on stdout is not Hermit's",
            stdout(1, &format!("{DEADLOCK_LINE}\n")),
            false,
            "violation: exit code 1".into(),
        ),
        (
            "report inside a line",
            stderr(1, "log: Deadlock detected: x\n"),
            false,
            "violation: exit code 1".into(),
        ),
    ]);
}

#[test]
fn hermits_own_failures() {
    let skid = "infra: hermit panicked on a PMU skid overshoot: the preemption landed late, so this schedule is not reproducible; raise `skid_margin`";
    let internal = "infra: hermit failed to run the program: hermit reported an internal failure";
    assert_table(vec![
        (
            "skid overshoot line",
            stderr(101, "SKID_OVERSHOOT at 12\n"),
            false,
            skid.into(),
        ),
        (
            "skid overshoot with a clean exit",
            stderr(0, "SKID_OVERSHOOT\n"),
            false,
            skid.into(),
        ),
        (
            "panicked and overshoot on different lines",
            stderr(101, "thread panicked\nthe overshoot was 3\n"),
            false,
            skid.into(),
        ),
        (
            "panicked alone",
            stderr(101, "thread panicked\n"),
            false,
            "violation: exit code 101".into(),
        ),
        (
            "overshoot alone",
            stderr(1, "an overshoot\n"),
            false,
            "violation: exit code 1".into(),
        ),
        (
            "skid overshoot on stdout is the guest's",
            stdout(1, "SKID_OVERSHOOT\n"),
            false,
            "violation: exit code 1".into(),
        ),
        (
            "usage error with an error line",
            stderr(2, "error: unknown flag --nope\nUsage: hermit run\n"),
            false,
            "infra: hermit rejected its command line (usage error): error: unknown flag --nope"
                .into(),
        ),
        (
            "usage error without an error line",
            stderr(2, "Usage: hermit run\n"),
            false,
            "infra: hermit rejected its command line (usage error): bad command line".into(),
        ),
        (
            "internal failure with an error line",
            stderr(1, "HERMIT_INTERNAL_FAILURE\nError: no ptrace\n"),
            false,
            "infra: hermit failed to run the program: Error: no ptrace".into(),
        ),
        (
            "internal failure without an error line",
            stderr(1, "HERMIT_INTERNAL_FAILURE\n"),
            false,
            internal.into(),
        ),
        (
            "internal failure outranks a violation marker",
            Trial {
                stderr: "HERMIT_INTERNAL_FAILURE\n".into(),
                ..stdout(1, "HAIRSPRING_VIOLATION: boom\n")
            },
            false,
            internal.into(),
        ),
        (
            "internal failure outranks the skid overshoot",
            stderr(101, "HERMIT_INTERNAL_FAILURE\nSKID_OVERSHOOT\n"),
            false,
            internal.into(),
        ),
        (
            "skid overshoot outranks a property failure",
            Trial {
                stderr: "SKID_OVERSHOOT\n".into(),
                ..with_events(&false_report(true), exited(1))
            },
            false,
            skid.into(),
        ),
    ]);
}

#[test]
fn a_program_hermit_could_not_start() {
    let refused = Trial {
        stderr: "Program ./guest does not exist or is not accessible\n".into(),
        ..exited(127)
    };
    let other_program = Trial {
        stderr: "Program ./other does not exist or is not accessible\n".into(),
        ..exited(127)
    };
    assert_table(vec![
        (
            "refused program",
            refused,
            false,
            "infra: target svc: program ./guest not found or not executable (Hermit: Program ./guest does not exist or is not accessible)".into(),
        ),
        (
            "another program's words",
            other_program,
            false,
            "violation: exit code 127".into(),
        ),
        (
            "exit 127 with no words",
            exited(127),
            false,
            "violation: exit code 127".into(),
        ),
    ]);
}

#[test]
fn guest_events_and_the_faults_around_them() {
    let failed = "violation: always 'x.id' was false 1 time(s) (proc p1, thread t, ordinal 4)";
    assert_table(vec![
        (
            "false property, clean exit",
            with_events(&false_report(true), exited(0)),
            false,
            failed.into(),
        ),
        (
            "false property, no end report",
            with_events(&false_report(false), exited(0)),
            false,
            failed.into(),
        ),
        (
            "false property outranks exit status and marker",
            Trial {
                stdout: "HAIRSPRING_VIOLATION: old\n".into(),
                ..with_events(&false_report(true), exited(9))
            },
            false,
            failed.into(),
        ),
        (
            "false property outranks a deadlock report",
            Trial {
                stderr: format!("{DEADLOCK_LINE}\n"),
                ..with_events(&false_report(true), exited(1))
            },
            false,
            failed.into(),
        ),
        (
            "true properties, clean exit",
            with_events(&true_report(true), exited(0)),
            false,
            "pass".into(),
        ),
        (
            "true properties, no end report, clean exit",
            with_events(&true_report(false), exited(0)),
            false,
            "inconclusive: process p1 sent no end report (cut off, or it crashed before reporting)"
                .into(),
        ),
        (
            "no end report with a failing exit is the exit status",
            with_events(&true_report(false), exited(3)),
            false,
            "violation: exit code 3".into(),
        ),
        (
            "config fault",
            with_fault(EventsFault::Config("bad".into()), exited(0)),
            false,
            "infra: bad".into(),
        ),
        (
            "config fault outranks a false read",
            with_fault(
                EventsFault::Config("bad".into()),
                with_events(&false_report(true), exited(0)),
            ),
            false,
            "infra: bad".into(),
        ),
        (
            "host fault",
            with_fault(EventsFault::Infra("disk".into()), exited(0)),
            false,
            "inconclusive: the guest's events could not be collected: disk".into(),
        ),
        (
            "host fault outranks a false read",
            with_fault(
                EventsFault::Infra("disk".into()),
                with_events(&false_report(true), exited(0)),
            ),
            false,
            "inconclusive: the guest's events could not be collected: disk".into(),
        ),
        (
            "host fault outranks a violation marker and the exit status",
            with_fault(
                EventsFault::Infra("disk".into()),
                stdout(1, "HAIRSPRING_VIOLATION: x\n"),
            ),
            false,
            "inconclusive: the guest's events could not be collected: disk".into(),
        ),
        (
            "guest fault, clean exit, events seen",
            with_fault(
                EventsFault::Guest("cut".into()),
                with_events(&true_report(true), exited(0)),
            ),
            false,
            "inconclusive: guest events are incomplete: cut".into(),
        ),
        (
            "guest fault, clean exit, no events",
            with_fault(EventsFault::Guest("cut".into()), exited(0)),
            false,
            "inconclusive: the output capture failed: cut".into(),
        ),
        (
            "guest fault with a false read",
            with_fault(
                EventsFault::Guest("cut".into()),
                with_events(&false_report(true), exited(0)),
            ),
            false,
            failed.into(),
        ),
        (
            "guest fault with a failing exit",
            with_fault(EventsFault::Guest("cut".into()), exited(4)),
            false,
            "violation: exit code 4".into(),
        ),
        (
            "guest fault with a marker",
            with_fault(
                EventsFault::Guest("cut".into()),
                stdout(0, "HAIRSPRING_VIOLATION: m\n"),
            ),
            false,
            "violation: m".into(),
        ),
        (
            "guest fault with a timeout, hang is violation",
            timed_out(with_fault(EventsFault::Guest("cut".into()), exited(0))),
            true,
            hang_as_violation(),
        ),
    ]);
}

#[test]
fn only_a_skid_overshoot_is_a_skid_overshoot() {
    let cases = [
        (stderr(101, "SKID_OVERSHOOT\n"), true),
        (stderr(101, "thread panicked\nan overshoot\n"), true),
        (stderr(101, "thread panicked\n"), false),
        (stderr(1, "an overshoot\n"), false),
        (stdout(1, "SKID_OVERSHOOT\n"), false),
        (exited(1), false),
    ];
    for (trial, want) in cases {
        assert_eq!(skid_overshoot(&trial), want, "{trial:?}");
    }
}
