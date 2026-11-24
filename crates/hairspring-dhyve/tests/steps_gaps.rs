//! The steps log has a row for every root command that was run or tried, and a module that is lost or changed ends a
//! session without a restore starting.
mod common;
use common::{quick, tiny_bounds, Rig};
use hairspring_dhyve::driver::{records_dir, run_session, steps_log, Plan};
use hairspring_dhyve::host::Timings;
use hairspring_dhyve::session::Session;
use hairspring_dhyve::vmname::Stamp;
use std::time::Duration;

const SEED: u64 = 1007;

fn session(rig: &Rig, helper_cap: Duration) -> Session {
    let mut host = rig.host.clone();
    host.session_stamp = Some(Stamp::parse("20261201T0540Z").unwrap());
    let timings = Timings {
        helper_cap,
        trial_cap: Duration::from_secs(1),
        resume_limit: Duration::from_millis(500),
        ..quick()
    };
    Session::new(host, timings, tiny_bounds())
        .with_one_prepare()
        .with_keep_all()
}

fn rows(session: &Session) -> Vec<Vec<String>> {
    std::fs::read_to_string(steps_log(&records_dir(session)))
        .unwrap()
        .lines()
        .map(|l| l.split('\t').map(String::from).collect())
        .collect()
}

fn rows_of(rows: &[Vec<String>], kind: &str) -> Vec<Vec<String>> {
    rows.iter().filter(|r| r[2] == kind).cloned().collect()
}

fn starts(rig: &Rig) -> usize {
    rig.calls()
        .iter()
        .filter(|c| c.starts_with("bhyve "))
        .count()
}

#[test]
fn a_destroy_that_times_out_still_writes_an_r5_row_with_rc_timeout_and_its_argv() {
    let rig = Rig::new("g-destroy-timeout");
    rig.ctl(&["destroy=hang"]);
    let s = session(&rig, Duration::from_millis(300));
    let summary = run_session(&s, &Plan::new(SEED, 1, 0));
    assert_eq!(summary.end, "END:PREPARE");
    let r5 = rows_of(&rows(&s), "R5");
    assert_eq!(r5.len(), 1, "{r5:?}");
    assert_eq!(r5[0][3], "done");
    assert_eq!(r5[0][5], "timeout");
    assert!(
        r5[0][6].starts_with("argv=") && r5[0][6].contains("--destroy"),
        "{:?}",
        r5[0]
    );
}

#[test]
fn a_checkpoint_that_times_out_still_writes_an_r6_row_with_rc_timeout_and_its_argv() {
    let rig = Rig::new("g-checkpoint-timeout");
    rig.ctl(&["checkpoint=hang"]);
    let s = session(&rig, Duration::from_millis(400));
    run_session(&s, &Plan::new(SEED, 1, 0));
    let r6 = rows_of(&rows(&s), "R6");
    assert_eq!(r6.len(), 1, "{r6:?}");
    assert_eq!(r6[0][5], "timeout");
    assert!(
        r6[0][6].starts_with("argv=") && r6[0][6].contains("--checkpoint"),
        "{:?}",
        r6[0]
    );
}

#[test]
fn a_command_that_cannot_be_run_at_all_writes_a_row_with_rc_dash() {
    let rig = Rig::new("g-destroy-missing");
    let mut host = rig.host.clone();
    host.sudo = rig.root.join("no-such-sudo");
    host.session_stamp = Some(Stamp::parse("20261201T0540Z").unwrap());
    let s = Session::new(host, quick(), tiny_bounds())
        .with_one_prepare()
        .with_keep_all();
    run_session(&s, &Plan::new(SEED, 1, 0));
    let r5 = rows_of(&rows(&s), "R5");
    assert_eq!(r5.len(), 1, "{r5:?}");
    assert_eq!(r5[0][5], "-");
    assert!(r5[0][6].starts_with("argv="));
}

#[test]
fn a_start_whose_spawn_fails_is_closed_by_a_done_row() {
    let rig = Rig::new("g-start-fails");
    rig.sudo_mode("vanish");
    let s = session(&rig, Duration::from_secs(5));
    let summary = run_session(&s, &Plan::new(SEED, 1, 0));
    assert_eq!(summary.end, "END:PREPARE");
    let r4 = rows_of(&rows(&s), "R4");
    let states: Vec<&str> = r4.iter().map(|r| r[3].as_str()).collect();
    assert_eq!(states, ["start", "done"], "{r4:?}");
    assert_eq!(r4[1][5], "-");
    assert_eq!(r4[0][4], r4[1][4], "the same VM");
}

#[test]
fn a_module_gone_before_the_prepare_ends_the_session_as_vmm_changed_and_starts_nothing() {
    let rig = Rig::new("g-prepare-no-module");
    rig.ctl(&["kld=absent"]);
    let s = session(&rig, Duration::from_secs(5));
    let summary = run_session(&s, &Plan::new(SEED, 1, 0));
    assert_eq!(summary.end, "END:VMM-CHANGED");
    assert_eq!(starts(&rig), 0);
}

#[test]
fn a_module_gone_or_changed_after_the_prepare_ends_the_session_with_no_further_start() {
    for (tag, setting) in [("gone", "kld_after=2"), ("changed", "kld_line_after=2")] {
        let rig = Rig::new(&format!("g-restore-{tag}"));
        rig.ctl(&[setting]);
        let s = session(&rig, Duration::from_secs(5));
        let summary = run_session(&s, &Plan::new(SEED, 1, 0));
        assert_eq!(summary.end, "END:VMM-CHANGED", "{tag}");
        assert_eq!(starts(&rig), 1, "{tag}: the prepare only");
        let trial_rows = rows_of(&rows(&s), "TRIAL");
        assert_eq!(
            trial_rows.len(),
            2,
            "{tag}: one start and one done, then the end: {trial_rows:?}"
        );
    }
}
