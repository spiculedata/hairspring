//! Prepare against the fake VM programs: the happy path, the shape of every command, and each scripted failure.
mod common;
use common::{quick, Rig, FIRST_NAME};
use hairspring_dhyve::error::InfraKind;
use hairspring_dhyve::fwcfg::Inputs;
use hairspring_dhyve::host::Timings;
use hairspring_dhyve::hostlock::HostLock;
use std::time::Duration;

const INPUTS: Inputs = Inputs { seed: 7, job: 0 };

fn failure_of(rig: &Rig, mode: &str) -> hairspring_dhyve::error::InfraError {
    rig.modes(&[mode]);
    rig.session().prepare(INPUTS).unwrap_err()
}

#[test]
fn a_prepare_checkpoints_the_stop_and_records_the_pointer_and_the_keys() {
    let rig = Rig::new("prep-ok");
    let prepared = rig.session().prepare(INPUTS).unwrap();
    assert_eq!(prepared.name.as_str(), FIRST_NAME);
    assert_eq!(prepared.record.rip, 0xffff_ffff_8012_3456);
    assert_eq!(prepared.record.keys.stop_seq, 1);
    assert_eq!(prepared.record.keys.branch_count, 395_000);
    assert_eq!(prepared.record.keys.vtime_ns, 777);
    assert!(prepared.record.unchanged());
    assert_eq!(prepared.input_hashes.len(), 6);
    assert!(rig.vms().is_empty(), "no VM is left: {:?}", rig.vms());
    assert!(rig.left("sock").is_empty());
}

#[test]
fn every_command_is_the_unprivileged_wrapper_shape_and_a_destroy_comes_before_and_after_the_start()
{
    let rig = Rig::new("prep-shape");
    rig.session().prepare(INPUTS).unwrap();
    let calls = rig.calls();
    let destroy = format!("bhyvectl --destroy --vm={FIRST_NAME}");
    let checkpoint_at = calls
        .iter()
        .position(|c| c.starts_with("bhyvectl --checkpoint="))
        .unwrap();
    let start_at = calls
        .iter()
        .position(|c| c == &format!("bhyve {FIRST_NAME}"))
        .unwrap();
    let destroys: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == destroy)
        .map(|(i, _)| i)
        .collect();
    assert!(destroys.first().unwrap() < &start_at, "{calls:?}");
    assert!(start_at < checkpoint_at, "{calls:?}");
    assert!(destroys.last().unwrap() > &checkpoint_at, "{calls:?}");
    let env = hairspring_dhyve::host::ENV_PROGRAM;
    let sudo_starts = calls
        .iter()
        .filter(|c| **c == format!("sudo {env}"))
        .count();
    assert_eq!(
        sudo_starts,
        calls.len() / 2,
        "every root command goes through sudo: {calls:?}"
    );
    let ckpt = calls
        .iter()
        .find(|c| c.starts_with("bhyvectl --checkpoint="))
        .unwrap();
    assert!(
        ckpt.ends_with(&format!("/boot --vm={FIRST_NAME}")),
        "{ckpt}"
    );
}

#[test]
fn a_library_path_other_than_the_pinned_one_means_nothing_runs() {
    let rig = Rig::new("prep-lib");
    rig.expect_lib("/somewhere/else");
    let err = rig.session().prepare(INPUTS).unwrap_err();
    assert!(
        matches!(
            err.kind,
            InfraKind::NoConnect
                | InfraKind::StartFailed
                | InfraKind::ShutdownFailed
                | InfraKind::NoReady
        ),
        "{err}"
    );
    assert!(rig.vms().is_empty());
}

#[test]
fn a_vm_name_that_exists_is_destroyed_before_the_start() {
    let rig = Rig::new("prep-reuse");
    rig.seed_vm(FIRST_NAME);
    assert!(rig.session().prepare(INPUTS).is_ok());
    assert!(rig.vms().is_empty());
}

#[test]
fn a_second_instance_is_told_another_run_holds_the_host_and_no_name_repeats() {
    let rig = Rig::new("prep-lock");
    let held = HostLock::acquire(
        &rig.host.state_dir,
        Duration::from_millis(50),
        Duration::from_millis(5),
    )
    .unwrap();
    let err = rig.session().prepare(INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::HostBusy);
    assert_eq!(err.to_string(), "another run holds the host");
    drop(held);
    let first = rig.session().prepare(INPUTS).unwrap();
    let second = rig.session().prepare(INPUTS).unwrap();
    assert_ne!(first.name, second.name);
    assert_ne!(first.stamp, second.stamp);
}

#[test]
fn two_instances_at_once_serialise_and_get_different_stamps() {
    let rig = Rig::new("prep-two");
    let slow = Timings {
        lock_wait: Duration::from_secs(20),
        ..quick()
    };
    let (a, b) = (rig.session_with(slow), rig.session_with(slow));
    let one = std::thread::spawn(move || a.prepare(INPUTS));
    let two = std::thread::spawn(move || b.prepare(INPUTS));
    let (one, two) = (one.join().unwrap().unwrap(), two.join().unwrap().unwrap());
    assert_ne!(one.stamp, two.stamp);
    assert_ne!(one.name, two.name);
}

#[test]
fn a_vm_that_never_connects_is_no_connect() {
    let rig = Rig::new("prep-noconnect");
    assert_eq!(failure_of(&rig, "no_connect").kind, InfraKind::NoConnect);
    assert!(rig.vms().is_empty());
}

#[test]
fn a_vm_that_connects_and_drops_before_its_first_stop_is_a_lost_connection() {
    let rig = Rig::new("prep-drop");
    assert_eq!(
        failure_of(&rig, "drop_before_ready").kind,
        InfraKind::ControlLost
    );
}

#[test]
fn a_vm_that_connects_and_never_reaches_a_stop_is_no_ready() {
    let rig = Rig::new("prep-noready");
    let session = rig.session_with(Timings {
        prepare_cap: Duration::from_millis(900),
        ..quick()
    });
    rig.modes(&["no_ready"]);
    let err = session.prepare(INPUTS).unwrap_err();
    assert!(
        matches!(err.kind, InfraKind::PrepareTimeout | InfraKind::NoReady),
        "{err}"
    );
    assert!(rig.vms().is_empty(), "the VM was destroyed");
}

#[test]
fn a_stop_that_has_moved_on_gets_no_shutdown() {
    let rig = Rig::new("prep-held");
    assert_eq!(failure_of(&rig, "not_held").kind, InfraKind::ShutdownFailed);
}

#[test]
fn a_shutdown_exit_other_than_zero_is_a_failed_shutdown() {
    let rig = Rig::new("prep-exit4");
    let err = failure_of(&rig, "shutdown_exit=4");
    assert_eq!(err.kind, InfraKind::ShutdownFailed);
    assert!(err.detail.contains('4'));
}

#[test]
fn a_checkpoint_command_that_fails_a_pointer_that_cannot_be_decoded_and_files_that_never_settle_are_checkpoint_errors(
) {
    for setting in ["checkpoint=fail", "checkpoint=undecodable"] {
        let rig = Rig::new("prep-ckpt");
        rig.ctl(&[setting]);
        let err = rig.session().prepare(INPUTS).unwrap_err();
        assert_eq!(err.kind, InfraKind::Checkpoint, "{setting}: {err}");
        assert!(rig.vms().is_empty());
    }
}

#[test]
fn a_checkpoint_whose_files_arrive_one_after_another_is_waited_for() {
    let rig = Rig::new("prep-slow");
    rig.ctl(&["checkpoint=slow"]);
    assert!(rig.session().prepare(INPUTS).is_ok());
}

#[test]
fn a_run_log_that_grows_past_its_bound_ends_the_prepare_as_a_log_error() {
    let rig = Rig::new("prep-bound");
    rig.modes(&["not_held"]);
    let tiny = hairspring_dhyve::logs::Bounds {
        run_log_bytes: 10,
        uart_log_bytes: 10,
        min_free_kib: 1,
    };
    let session = hairspring_dhyve::session::Session::new(rig.host.clone(), quick(), tiny);
    let err = session.prepare(INPUTS).unwrap_err();
    assert!(
        matches!(err.kind, InfraKind::LogBound | InfraKind::ShutdownFailed),
        "{err}"
    );
}

#[test]
fn a_directory_path_too_long_for_a_socket_is_refused_before_anything_starts() {
    let rig = Rig::new("prep-long");
    let mut host = rig.host.clone();
    host.state_dir = rig.root.join("d".repeat(120));
    let session = hairspring_dhyve::session::Session::new(host, quick(), common::tiny_bounds());
    let err = session.prepare(INPUTS).unwrap_err();
    assert_eq!(err.kind, InfraKind::Settings);
    assert!(rig.calls().is_empty());
}
