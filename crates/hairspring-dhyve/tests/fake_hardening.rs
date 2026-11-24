//! The run path against a fake that misbehaves in the ways a real host can: a VM that does not go when it is destroyed, a
//! `sudo` that cannot be signalled, a destroy that never answers, a module that is not loaded, and a panic. None of them
//! may leave the backend waiting for ever, or leave a VM, a child process or a control socket behind.
mod common;
use common::{quick, Rig, FIRST_NAME};
use hairspring_dhyve::director::ResumeResult;
use hairspring_dhyve::error::{InfraError, InfraKind};
use hairspring_dhyve::fwcfg::Inputs;
use hairspring_dhyve::host::Timings;
use hairspring_dhyve::judge::judge_trial;
use hairspring_dhyve::prepare::Prepared;
use hairspring_dhyve::session::Session;
use hairspring_dhyve::signal::Terminator;
use std::process::Command;
use std::time::{Duration, Instant};

const INPUTS: Inputs = Inputs { seed: 7, job: 0 };

fn short_cap() -> Timings {
    Timings {
        trial_cap: Duration::from_millis(400),
        ..quick()
    }
}

fn prepared(rig: &Rig, session: &Session) -> Prepared {
    let p = session.prepare(INPUTS).unwrap();
    rig.modes(&["", "hang ignore_destroy"]);
    p
}

fn bhyve_starts(rig: &Rig) -> usize {
    rig.calls()
        .iter()
        .filter(|c| c.starts_with("bhyve "))
        .count()
}

fn destroys(rig: &Rig) -> usize {
    rig.calls()
        .iter()
        .filter(|c| c.contains("--destroy"))
        .count()
}

fn processes_mentioning(text: &str) -> usize {
    let out = Command::new("/bin/ps")
        .args(["-ax", "-o", "command"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(text))
        .count()
}

// ---- M2: every wait after a destroy or a kill has a deadline ---------------------------------------------------------

#[test]
fn a_vm_that_ignores_destroy_and_a_kill_that_is_refused_end_in_an_error_within_a_bound() {
    let rig = Rig::new("h-refused");
    let session = rig
        .session_with(short_cap())
        .with_terminator(Terminator::AlwaysRefused);
    let prepared = prepared(&rig, &session);
    rig.ctl(&["destroy=fail"]);
    let started = Instant::now();
    let err = session.restore_trial(&prepared, 0, INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::VmDidNotExit, "{err}");
    assert_eq!(err.to_string(), "the VM program did not exit");
    assert!(
        started.elapsed() < Duration::from_millis(2900),
        "{:?}",
        started.elapsed()
    );
    assert!(destroys(&rig) >= 1, "the destroy was still tried");
}

#[test]
fn a_sudo_that_ignores_term_is_ended_by_the_kill_and_a_destroy_that_failed_is_an_error() {
    let rig = Rig::new("h-sudo");
    let session = rig.session_with(short_cap());
    let prepared = prepared(&rig, &session);
    rig.sudo_mode("ignore_term");
    rig.ctl(&["destroy=fail"]);
    let started = Instant::now();
    let err = session.restore_trial(&prepared, 0, INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::VmDidNotExit, "{err}");
    assert!(err.detail.contains("destroy"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_destroy_that_never_answers_is_a_bounded_error_and_nothing_is_started() {
    let rig = Rig::new("h-destroy-hang");
    rig.ctl(&["destroy=hang"]);
    let session = rig.session_with(Timings {
        helper_cap: Duration::from_millis(300),
        ..quick()
    });
    let started = Instant::now();
    let err = session.prepare(INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::StartFailed, "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(bhyve_starts(&rig), 0);
}

#[test]
fn a_destroy_that_ends_the_vm_leaves_two_destroys_for_the_start_and_no_more() {
    let rig = Rig::new("h-count");
    let session = rig.session_with(short_cap());
    let prepared = session.prepare(INPUTS).unwrap();
    assert_eq!(
        destroys(&rig),
        2,
        "one before the prepare's start and one after: {:?}",
        rig.calls()
    );
    rig.modes(&["", "hang"]);
    let before = destroys(&rig);
    let run = session.restore_trial(&prepared, 0, INPUTS).unwrap();
    assert_eq!(
        destroys(&rig) - before,
        2,
        "a capped trial is still two: {:?}",
        rig.calls()
    );
    assert!(judge_trial(&run, 1, false).is_ok());
}

// ---- M4: cleanup on a panic -----------------------------------------------------------------------------------------

#[test]
fn a_panic_while_waiting_leaves_no_vm_no_process_and_no_control_socket() {
    let rig = Rig::new("h-panic");
    let session = rig.session().with_panic_in_wait();
    let marker = rig.root.display().to_string();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.prepare(INPUTS)));
    assert!(result.is_err(), "the test hook panics");
    assert!(rig.vms().is_empty(), "a VM is left: {:?}", rig.vms());
    assert!(rig.left("sock").is_empty(), "a control socket is left");
    assert_eq!(processes_mentioning(&marker), 0, "a process is left");
    assert!(destroys(&rig) >= 2, "the guard destroyed the VM");
}

#[test]
fn the_lock_is_free_again_after_a_panic() {
    let rig = Rig::new("h-panic-lock");
    let session = rig.session().with_panic_in_wait();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.prepare(INPUTS)));
    assert!(rig.session().prepare(INPUTS).is_ok());
}

// ---- M3: the vmm module is checked before every start --------------------------------------------------------------

#[test]
fn a_prepare_with_no_module_loaded_starts_nothing() {
    let rig = Rig::new("h-kld-prepare");
    rig.ctl(&["kld=absent"]);
    let err = rig.session().prepare(INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::VmmNotLoaded);
    assert_eq!(bhyve_starts(&rig), 0);
    assert!(rig.calls().iter().any(|c| c.starts_with("kldstat")));
}

#[test]
fn a_restore_with_no_module_loaded_starts_nothing_and_a_different_module_is_refused() {
    let rig = Rig::new("h-kld-trial");
    let session = rig.session();
    let prepared = session.prepare(INPUTS).unwrap();
    rig.modes(&[""]);
    let starts = bhyve_starts(&rig);
    rig.ctl(&["kld=absent"]);
    let err = session.restore_trial(&prepared, 0, INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::VmmNotLoaded);
    rig.ctl(&["kld_line= 9 1 0xffffffff83000000 4b000 vmm.ko"]);
    let err = session.restore_trial(&prepared, 0, INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::VmmChanged);
    assert_eq!(bhyve_starts(&rig), starts, "no start in either case");
}

// ---- H8: pins -------------------------------------------------------------------------------------------------------

#[test]
fn a_symbolic_link_for_a_vm_program_is_refused_before_anything_is_spawned() {
    let rig = Rig::new("h-link");
    let mut host = rig.host.clone();
    host.require_regular_files = true;
    let session = Session::new(host, quick(), common::tiny_bounds());
    let err = session.prepare(INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::PinMismatch);
    assert!(rig.calls().is_empty());
}

#[test]
fn a_program_that_differs_from_its_pin_is_refused_before_anything_is_spawned() {
    let rig = Rig::new("h-pin");
    let mut host = rig.host.clone();
    host.pins.bhyve = Some("0".repeat(64));
    let err = Session::new(host, quick(), common::tiny_bounds())
        .prepare(INPUTS)
        .unwrap_err();
    assert_eq!(err.kind, InfraKind::PinMismatch);
    assert!(rig.calls().is_empty());
}

// ---- N1, N5, N3, N4: what a restore shows ---------------------------------------------------------------------------

fn trial_error(name: &str, mode: &str) -> (Rig, InfraError) {
    let rig = Rig::new(name);
    let session = rig.session_with(Timings {
        resume_limit: Duration::from_millis(500),
        ..quick()
    });
    let prepared = session.prepare(INPUTS).unwrap();
    rig.modes(&["", mode]);
    let result = session
        .restore_trial(&prepared, 0, INPUTS)
        .and_then(|run| judge_trial(&run, 8, false));
    let err = result.unwrap_err();
    (rig, err)
}

#[test]
fn a_stop_after_the_resume_followed_by_the_vms_exit_is_named_and_not_called_a_missing_exit_status()
{
    let (_rig, err) = trial_error("h-stop-exit", "stop_then_exit");
    assert_eq!(err.kind, InfraKind::StoppedAgain);
    assert!(err.detail.contains("stop kind 1"), "{err}");
    assert!(!err.to_string().contains("branch limit"));
}

#[test]
fn a_replayed_stop_that_differs_from_the_record_is_a_fast_error() {
    let started = Instant::now();
    let (_rig, err) = trial_error("h-ready-differs", "ready_differs");
    assert_eq!(err.kind, InfraKind::ResumeNotTaken);
    assert!(
        err.detail.contains("differs from the checkpoint record"),
        "{err}"
    );
    assert!(started.elapsed() < Duration::from_secs(8));
}

#[test]
fn a_stop_event_as_the_first_event_of_a_restore_is_no_ready_and_not_a_stop_again() {
    let (_rig, err) = trial_error("h-stopped-first", "stopped_first");
    assert_eq!(err.kind, InfraKind::NoReady, "{err}");
}

#[test]
fn a_hostile_log_line_reaches_no_error_text() {
    for mode in ["stale_stop hostile_line", "hostile_other"] {
        let (rig, err) = trial_error(&format!("h-hostile-{}", mode.len()), mode);
        let text = err.to_string();
        assert_eq!(err.kind, InfraKind::ResumeNotTaken);
        for bad in ["deterministic", "\u{1b}", "-o x=y", "\u{fffd}"] {
            assert!(!text.contains(bad), "{bad:?} in {text:?}");
        }
        assert!(text.bytes().all(|b| (0x20..0x7f).contains(&b)), "{text:?}");
        drop(rig);
    }
}

#[test]
fn a_link_at_the_serial_log_is_refused_and_not_followed() {
    let (_rig, err) = trial_error("h-uart-link", "uart_link");
    assert_eq!(err.kind, InfraKind::SerialMissing);
}

#[test]
fn a_fifo_at_the_serial_log_is_refused_without_blocking() {
    let started = Instant::now();
    let (_rig, err) = trial_error("h-uart-fifo", "uart_fifo");
    assert_eq!(err.kind, InfraKind::SerialMissing);
    assert!(started.elapsed() < Duration::from_secs(6));
}

// ---- H4: keep everything --------------------------------------------------------------------------------------------

#[test]
fn a_session_that_keeps_everything_keeps_the_prepares_log_and_every_trials_logs() {
    let rig = Rig::new("h-keep");
    let session = rig.session().with_keep_all();
    let prepared = session.prepare(INPUTS).unwrap();
    assert!(
        prepared.prepare_run.run_log.exists(),
        "the prepare's run log is kept"
    );
    assert!(prepared.prepare_run.fwcfg_dir.exists());
    rig.modes(&["", ""]);
    let run = session.restore_trial(&prepared, 0, INPUTS).unwrap();
    let judged = judge_trial(&run, 8, false).unwrap();
    session.clean_trial(
        &run,
        matches!(judged.verdict, hairspring_engine::verdict::Verdict::Pass),
    );
    assert!(run.run.run_log.exists() && run.run.uart_log.exists());
    assert!(run.run.fwcfg_dir.exists(), "the inputs of a pass are kept");
    let plain = Rig::new("h-keep-not");
    let session = plain.session();
    let prepared = session.prepare(INPUTS).unwrap();
    assert!(!prepared.prepare_run.dir.exists());
}

#[test]
fn a_second_prepare_in_a_session_that_allows_one_is_refused_before_anything_starts() {
    let rig = Rig::new("h-reprepare");
    let session = rig.session().with_one_prepare();
    session.prepare(INPUTS).unwrap();
    let calls = rig.calls().len();
    let err = session.prepare(INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::SecondPrepare);
    assert_eq!(rig.calls().len(), calls);
}

// ---- H2: names ------------------------------------------------------------------------------------------------------

#[test]
fn the_session_stamp_is_used_with_run_numbers_from_r01_and_restores_from_r02() {
    let rig = Rig::new("h-names");
    let mut host = rig.host.clone();
    host.session_stamp = Some(hairspring_dhyve::vmname::Stamp::parse("20261201T0540Z").unwrap());
    let session = Session::new(host, quick(), common::tiny_bounds());
    let prepared = session.prepare(INPUTS).unwrap();
    assert_eq!(prepared.name.as_str(), "vmvx_20261201T0540Z_XB_r01");
    assert!(
        prepared.record.dir.ends_with("ckpt"),
        "{:?}",
        prepared.record.dir
    );
    rig.modes(&[""]);
    let run = session.restore_trial(&prepared, 0, INPUTS).unwrap();
    assert_eq!(run.name.as_str(), "vmvx_20261201T0540Z_XB_r02");
    let last = session.restore_trial(&prepared, 14, INPUTS).unwrap();
    assert_eq!(last.name.as_str(), "vmvx_20261201T0540Z_XB_r16");
    let none_left = session.restore_trial(&prepared, 98, INPUTS).unwrap_err();
    assert_eq!(none_left.kind, InfraKind::Settings);
}

#[test]
fn a_prepare_is_never_named_r00() {
    let rig = Rig::new("h-r00");
    let prepared = rig.session().prepare(INPUTS).unwrap();
    assert_eq!(prepared.name.as_str(), FIRST_NAME);
    assert!(!prepared.name.as_str().ends_with("r00"));
}

#[test]
fn a_start_runs_under_cpuset_on_the_pinned_core() {
    let rig = Rig::new("h-cpuset");
    rig.session().prepare(INPUTS).unwrap();
    let calls = rig.calls();
    let at = calls
        .iter()
        .position(|c| c == "cpuset -l 1")
        .expect("cpuset was used");
    assert!(calls[at + 1].starts_with("bhyve "), "{calls:?}");
}

#[test]
fn a_checkpoint_made_by_another_module_is_refused() {
    let rig = Rig::new("h-module");
    rig.ctl(&["checkpoint=old_module"]);
    let err = rig.session().prepare(INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::ModuleMismatch);
    assert_eq!(
        err.to_string(),
        "the checkpoint was not made by the expected module"
    );
}

#[test]
fn a_resume_that_was_not_taken_names_only_the_outcome() {
    let (_rig, err) = trial_error("h-outcome", "other_line");
    assert_eq!(err.kind, InfraKind::ResumeNotTaken);
    assert_eq!(err.detail, "another line came first");
    let _ = ResumeResult::NotSent;
}
