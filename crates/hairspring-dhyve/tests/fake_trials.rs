//! Restores against the fake VM programs: what a trial sees, how it is judged, and each scripted failure, which is an
//! infrastructure error and never a finding.
mod common;
use common::{quick, Rig};
use hairspring_dhyve::director::ResumeResult;
use hairspring_dhyve::error::{InfraError, InfraKind};
use hairspring_dhyve::fwcfg::Inputs;
use hairspring_dhyve::host::Timings;
use hairspring_dhyve::judge::{judge_trial, observe, Judged};
use hairspring_dhyve::prepare::Prepared;
use hairspring_dhyve::session::Session;
use hairspring_engine::verdict::Verdict;
use std::time::Duration;

const INPUTS: Inputs = Inputs { seed: 7, job: 0 };
const CAP_SECS: u64 = 8;

/// A prepared rig whose next start of the fake `bhyve` follows `mode`.
fn ready(name: &str, mode: &str) -> (Rig, Session, Prepared) {
    let rig = Rig::new(name);
    let session = rig.session();
    let prepared = session.prepare(INPUTS).unwrap();
    rig.modes(&["", mode]);
    (rig, session, prepared)
}

fn trial(
    session: &Session,
    prepared: &Prepared,
    ordinal: u32,
    seed: u64,
) -> Result<Judged, InfraError> {
    let run = session.restore_trial(prepared, ordinal, INPUTS.with_seed(seed))?;
    judge_trial(&run, CAP_SECS, false)
}

fn infra(name: &str, mode: &str) -> InfraError {
    let (rig, session, prepared) = ready(name, mode);
    let err = trial(&session, &prepared, 0, 7).unwrap_err();
    assert!(
        rig.vms().is_empty(),
        "{mode}: a VM is left: {:?}",
        rig.vms()
    );
    assert!(rig.left("sock").is_empty(), "{mode}: a socket is left");
    err
}

#[test]
fn a_restore_is_resumed_reads_its_exit_report_and_passes() {
    let (rig, session, prepared) = ready("t-ok", "");
    let run = session.restore_trial(&prepared, 0, INPUTS).unwrap();
    assert_eq!(run.control.resume, ResumeResult::Seen);
    assert_eq!(
        run.control.skipped, 2,
        "the debug exit at the restored pointer and the start time line"
    );
    let seen = observe(&run);
    assert!(seen.resume_seen && seen.exit_reported);
    let judged = judge_trial(&run, CAP_SECS, false).unwrap();
    assert_eq!(judged.verdict, Verdict::Pass);
    assert_eq!(judged.exit_code, Some(0));
    assert!(rig.vms().is_empty());
}

#[test]
fn a_restore_with_the_checkpointed_inputs_and_one_with_another_seed_file_each_read_their_own() {
    let (rig, session, prepared) = ready("t-seeds", "");
    rig.modes(&["", "", ""]);
    trial(&session, &prepared, 0, 7).unwrap();
    trial(&session, &prepared, 1, 8).unwrap();
    let seeds = rig.trials();
    assert!(seeds[0].starts_with("seed=7 "), "{seeds:?}");
    assert!(seeds[1].starts_with("seed=8 "), "{seeds:?}");
    let names: Vec<&str> = seeds
        .iter()
        .map(|s| s.split("name=").nth(1).unwrap())
        .collect();
    assert_ne!(names[0], names[1], "each trial has its own name");
}

#[test]
fn serial_output_and_event_lines_of_equal_restores_are_equal() {
    let (rig, session, prepared) = ready("t-equal", "");
    rig.modes(&["", "", ""]);
    rig.events(&["plain output"]);
    let one = session.restore_trial(&prepared, 0, INPUTS).unwrap();
    let two = session.restore_trial(&prepared, 1, INPUTS).unwrap();
    assert_eq!(observe(&one).serial_digest, observe(&two).serial_digest);
    assert_eq!(observe(&one).events_digest, observe(&two).events_digest);
    let other = session
        .restore_trial(&prepared, 2, INPUTS.with_seed(99))
        .unwrap();
    assert!(observe(&other).resume_seen);
}

#[test]
fn a_guest_that_exits_non_zero_or_prints_a_violation_marker_is_a_finding() {
    let (_rig, session, prepared) = ready("t-exit", "exit=3");
    let judged = trial(&session, &prepared, 0, 7).unwrap();
    assert_eq!(
        judged.verdict,
        Verdict::Violation("exit code 3".to_string())
    );
    let (rig, session, prepared) = ready("t-marker", "");
    rig.events(&["HAIRSPRING_VIOLATION: balance went negative at seed {seed}"]);
    let judged = trial(&session, &prepared, 0, 7).unwrap();
    assert_eq!(
        judged.verdict,
        Verdict::Violation("balance went negative at seed 7".to_string())
    );
}

#[test]
fn a_guest_ended_by_a_signal_is_a_finding_with_no_exit_code() {
    let (_rig, session, prepared) = ready("t-signal", "signal=9");
    let judged = trial(&session, &prepared, 0, 7).unwrap();
    assert_eq!(judged.signal, Some(9));
    assert_eq!(
        judged.verdict,
        Verdict::Violation("terminated by signal 9".to_string())
    );
}

#[test]
fn a_trial_that_reaches_the_time_cap_with_no_exit_report_is_a_hang_and_never_a_pass() {
    let rig = Rig::new("t-hang");
    let session = rig.session_with(Timings {
        trial_cap: Duration::from_millis(400),
        ..quick()
    });
    let prepared = session.prepare(INPUTS).unwrap();
    rig.modes(&["", "hang"]);
    let run = session.restore_trial(&prepared, 0, INPUTS).unwrap();
    let judged = judge_trial(&run, 1, false).unwrap();
    assert!(
        matches!(judged.verdict, Verdict::Hang(_)),
        "{:?}",
        judged.verdict
    );
    let as_finding = judge_trial(&run, 1, true).unwrap();
    assert!(matches!(as_finding.verdict, Verdict::Violation(_)));
    assert!(rig.vms().is_empty(), "the VM was destroyed at the cap");
}

#[test]
fn a_stale_stop_sequence_means_the_resume_was_not_taken() {
    let err = infra("t-stale", "stale_stop");
    assert_eq!(err.kind, InfraKind::ResumeNotTaken);
    assert!(err.detail.contains("refused"), "{err}");
}

#[test]
fn another_line_first_after_the_stop_means_the_resume_was_not_taken() {
    assert_eq!(
        infra("t-other", "other_line").kind,
        InfraKind::ResumeNotTaken
    );
}

#[test]
fn a_dropped_socket_after_the_resume_means_the_resume_was_not_taken() {
    assert_eq!(
        infra("t-dropped", "drop_after_resume").kind,
        InfraKind::ResumeNotTaken
    );
}

#[test]
fn a_restored_pointer_the_log_does_not_show_leaves_a_debug_line_that_decides_the_state() {
    assert_eq!(
        infra("t-rip", "rip_mismatch").kind,
        InfraKind::ResumeNotTaken
    );
}

#[test]
fn a_restore_with_no_skipped_lines_is_still_taken() {
    let (_rig, session, prepared) = ready("t-noskip", "no_skipped_lines");
    assert_eq!(
        trial(&session, &prepared, 0, 7).unwrap().verdict,
        Verdict::Pass
    );
}

#[test]
fn a_stopped_event_during_the_trial_is_a_stop_again() {
    let err = infra("t-limit-open", "limit_open");
    assert_eq!(err.kind, InfraKind::StoppedAgain);
    assert_eq!(err.to_string(), "the VM stopped again after it was resumed");
}

#[test]
fn a_stop_logged_after_the_connect_line_is_a_stop_again_and_not_a_hang() {
    let rig = Rig::new("t-limit-log");
    let session = rig.session_with(Timings {
        trial_cap: Duration::from_millis(800),
        ..quick()
    });
    let prepared = session.prepare(INPUTS).unwrap();
    rig.modes(&["", "limit_log"]);
    let run = session.restore_trial(&prepared, 0, INPUTS).unwrap();
    assert_eq!(
        judge_trial(&run, 1, false).unwrap_err().kind,
        InfraKind::StoppedAgain
    );
}

#[test]
fn a_log_past_its_bound_ends_the_trial_as_a_log_error() {
    let err = infra("t-overrun", "log_overrun");
    assert_eq!(err.kind, InfraKind::LogBound);
    assert_eq!(err.to_string(), "log size bound reached");
}

#[test]
fn a_missing_empty_or_unfinished_serial_log_is_an_infrastructure_error() {
    assert_eq!(infra("t-nouart", "no_uart").kind, InfraKind::SerialMissing);
    let (rig, session, prepared) = ready("t-noexit", "no_exit");
    rig.events(&["only output"]);
    assert_eq!(
        trial(&session, &prepared, 0, 7).unwrap_err().kind,
        InfraKind::NoExitStatus
    );
}

#[test]
fn a_changed_checkpoint_stops_the_restore_before_a_vm_starts() {
    let (rig, session, prepared) = ready("t-changed", "");
    std::fs::write(prepared.record.dir.join("boot"), b"changed").unwrap();
    let before = rig.calls().len();
    let err = session.restore_trial(&prepared, 0, INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::Checkpoint);
    assert_eq!(rig.calls().len(), before, "nothing was started");
}

#[test]
fn a_vm_that_never_connects_in_a_restore_is_no_connect() {
    let err = infra("t-noconn", "no_connect");
    assert_eq!(err.kind, InfraKind::NoConnect);
}

#[test]
fn a_name_that_exists_is_destroyed_before_the_restore_starts() {
    let (rig, session, prepared) = ready("t-reuse", "");
    rig.seed_vm("vmvx_00000000T0001Z_XB_r02");
    assert_eq!(
        trial(&session, &prepared, 0, 7).unwrap().verdict,
        Verdict::Pass
    );
}

#[test]
fn more_than_a_stamp_of_trials_move_to_a_fresh_stamp() {
    let rig = Rig::new("t-stamps");
    let session = rig.session();
    let prepared = session.prepare(INPUTS).unwrap();
    rig.modes(&[""]);
    let late = session.restore_trial(&prepared, 150, INPUTS).unwrap();
    assert_ne!(late.stamp, prepared.stamp);
    assert!(late.name.as_str().ends_with("_XB_r01"));
}

#[test]
fn a_clean_trial_leaves_nothing_and_a_finding_keeps_its_logs() {
    let (rig, session, prepared) = ready("t-clean", "");
    let run = session.restore_trial(&prepared, 0, INPUTS).unwrap();
    session.clean_trial(&run, false);
    assert!(!run.run.dir.exists());
    rig.modes(&["", "", "exit=1"]);
    let run = session.restore_trial(&prepared, 1, INPUTS).unwrap();
    session.clean_trial(&run, true);
    assert!(run.run.run_log.exists() && run.run.uart_log.exists());
    assert!(!run.run.fwcfg_dir.exists());
}
