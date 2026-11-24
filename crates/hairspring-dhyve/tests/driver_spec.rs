//! The session driver against the session's contract: the steps log in its fixed form with a row for every root command,
//! the prepare's record, a record for a restore that errors, the probe gate, the input job, the RDRAND gate, the END codes,
//! the start rule before the prepare, what counts, and what the records keep whole.
mod common;
use common::Rig;
use hairspring_dhyve::driver::{end_code, records_dir, run_session, steps_log, Plan};
use hairspring_dhyve::error::{InfraError, InfraKind};
use hairspring_dhyve::session::Session;
use hairspring_dhyve::vmname::Stamp;
use hairspring_engine::probe::{ProbeReport, ProbeRow, ProbeStatus};
use serde_json::Value;
use std::time::{Duration, Instant};

const STAMP: &str = "20261201T0540Z";

fn session(rig: &Rig) -> Session {
    let mut host = rig.host.clone();
    host.session_stamp = Some(Stamp::parse(STAMP).unwrap());
    Session::new(host, common::quick(), common::tiny_bounds())
        .with_one_prepare()
        .with_keep_all()
}

fn plan(control: u32, other: u32) -> Plan {
    Plan::new(1007, control, other)
}

fn step_rows(_rig: &Rig, session: &Session) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(steps_log(&records_dir(session))).unwrap_or_default();
    text.lines()
        .map(|l| l.split('\t').map(String::from).collect())
        .collect()
}

fn record(rig: &Rig, session: &Session, name: &str) -> Value {
    let path = records_dir(session).join(format!("{name}.json"));
    let _ = rig;
    serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap()
}

fn ready_probe() -> ProbeReport {
    ProbeReport {
        subject: "dhyve backend (experimental)".into(),
        rows: vec![
            ProbeRow {
                label: "cpu".into(),
                status: ProbeStatus::Ok,
                detail: "x".into(),
            },
            ProbeRow {
                label: "module".into(),
                status: ProbeStatus::Ok,
                detail: "x".into(),
            },
            ProbeRow {
                label: "module kind".into(),
                status: ProbeStatus::Note,
                detail: "x".into(),
            },
            ProbeRow {
                label: "access".into(),
                status: ProbeStatus::Note,
                detail: "x".into(),
            },
            ProbeRow {
                label: "nmi".into(),
                status: ProbeStatus::Ok,
                detail: "x".into(),
            },
        ],
        ready_note: "ok".into(),
        missing: Vec::new(),
    }
}

fn failing_probe() -> ProbeReport {
    ProbeReport {
        rows: vec![
            ProbeRow {
                label: "module".into(),
                status: ProbeStatus::Fail,
                detail: "x".into(),
            },
            ProbeRow {
                label: "leftovers".into(),
                status: ProbeStatus::Fail,
                detail: "x".into(),
            },
        ],
        missing: vec!["no module".into()],
        ..ready_probe()
    }
}

// ---- OI-1: the steps log -----------------------------------------------------------------------------------------------

#[test]
fn the_steps_log_has_seven_fields_a_utc_time_and_a_row_for_every_root_command() {
    let rig = Rig::new("s-steps");
    let session = session(&rig);
    let summary = run_session(&session, &plan(2, 1));
    assert!(summary.clean(), "{}", summary.to_json());
    let rows = step_rows(&rig, &session);
    for r in &rows {
        assert_eq!(r.len(), 7, "{r:?}");
        assert_eq!(r[0].len(), 20);
        assert!(r[0].ends_with('Z') && r[0].as_bytes()[10] == b'T', "{r:?}");
        assert_eq!(r[1], "driver");
    }
    let count = |kind: &str, state: &str| {
        rows.iter()
            .filter(|r| r[2] == kind && r[3] == state)
            .count()
    };
    let calls = rig.calls();
    assert_eq!(count("R4", "start"), 4, "a prepare and three restores");
    assert_eq!(count("R4", "done"), 4);
    assert_eq!(
        count("R5", "done"),
        calls.iter().filter(|c| c.contains("--destroy")).count()
    );
    assert_eq!(count("R6", "done"), 1);
    assert_eq!(
        (count("PREPARE", "start"), count("PREPARE", "done")),
        (1, 1)
    );
    assert_eq!((count("TRIAL", "start"), count("TRIAL", "done")), (3, 3));
    assert_eq!(rows.last().unwrap()[2..], ["END", "row", "-", "-", "DONE"]);
    let r4 = rows
        .iter()
        .find(|r| r[2] == "R4" && r[3] == "start")
        .unwrap();
    assert_eq!(r4[4], format!("vmvx_{STAMP}_XB_r01"));
    assert_eq!(r4[5], "running");
    assert!(r4[6].starts_with("argv="), "{}", r4[6]);
    assert!(
        r4[6].contains(" -n /usr/bin/env -i LD_LIBRARY_PATH="),
        "{}",
        r4[6]
    );
    assert!(
        r4[6].contains("<W>/") && !r4[6].contains(&rig.host.state_dir.display().to_string()),
        "{}",
        r4[6]
    );
    assert!(r4[6].ends_with(&format!("vmvx_{STAMP}_XB_r01")));
    let done = rows
        .iter()
        .find(|r| r[2] == "R4" && r[3] == "done")
        .unwrap();
    assert_eq!((done[5].as_str(), done[6].as_str()), ("0", "-"));
}

// ---- OI-2: the prepare's record and the record of a restore that errors -------------------------------------------------

#[test]
fn the_prepare_has_a_record_with_its_hashes_and_restore_keys() {
    let rig = Rig::new("s-prep-record");
    let session = session(&rig);
    run_session(&session, &plan(1, 0));
    let r = record(&rig, &session, "prepare");
    assert_eq!(r["name"], format!("vmvx_{STAMP}_XB_r01"));
    assert_eq!(r["stamp"], STAMP);
    assert_eq!(r["checkpoint_sha256"].as_array().unwrap().len(), 3);
    assert_eq!(r["restored_pointer"], "0xffffffff80123456");
    assert_eq!(r["restore_keys"]["stop_seq"], 1);
    assert_eq!(r["restore_keys"]["branch_count"], 395_000);
    assert_eq!(r["restore_keys"]["vtime_ns"], 777);
    assert_eq!(
        r["fwcfg_sha256"]["job"],
        "ad8324689b78cd6e2177b4f02dec255566ae4a430a820312ee7ca567e32db079"
    );
    assert_eq!(
        r["fwcfg_sha256"]["seed"],
        "292b1fcd6c00d6ff28d8ca5cb6d02f9470e36712a39fb0135b9af7114df4ed7a"
    );
    assert_eq!(r["argv_masked_sha256"].as_str().unwrap().len(), 64);
    assert!(r["seconds"].is_number());
}

#[test]
fn a_restore_that_returns_an_error_still_has_a_record() {
    let rig = Rig::new("s-error-record");
    rig.modes(&["", "", "stale_stop"]);
    let session = session(&rig);
    let summary = run_session(&session, &plan(2, 0));
    assert_eq!(summary.control.errors.len(), 1);
    let r = record(&rig, &session, &format!("vmvx_{STAMP}_XB_r03"));
    assert_eq!(r["verdict"], "infra");
    assert_eq!(r["group"], "C");
    assert_eq!(r["position"], 2);
    assert!(r["reason"].as_str().unwrap().contains("resume"), "{r}");
}

// ---- OI-3: the probe -----------------------------------------------------------------------------------------------------

#[test]
fn a_probe_that_fails_ends_the_session_before_anything_is_started() {
    let rig = Rig::new("s-probe-fail");
    let session = session(&rig);
    let mut p = plan(2, 1);
    p.probe = Some(failing_probe);
    let summary = run_session(&session, &p);
    assert_eq!(summary.end, "END:PROBE");
    assert!(rig.calls().is_empty());
    let rows = step_rows(&rig, &session);
    let probe = rows.iter().find(|r| r[2] == "PROBE").unwrap();
    assert_eq!(probe[3], "row");
    assert!(
        probe[6].starts_with("gate=FAIL:module,leftovers"),
        "{}",
        probe[6]
    );
    assert_eq!(rows.last().unwrap()[6], "END:PROBE");
}

#[test]
fn a_probe_that_passes_is_recorded_with_the_rows_it_does_not_gate_on() {
    let rig = Rig::new("s-probe-pass");
    let session = session(&rig);
    let mut p = plan(1, 0);
    p.probe = Some(ready_probe);
    assert!(run_session(&session, &p).clean());
    let rows = step_rows(&rig, &session);
    let probe = rows.iter().find(|r| r[2] == "PROBE").unwrap();
    assert_eq!(
        probe[6],
        "gate=PASS cpu=ok module_kind=note access=note nmi=ok"
    );
}

// ---- OI-4: the job input ------------------------------------------------------------------------------------------------

#[test]
fn the_job_file_of_every_run_is_one() {
    let rig = Rig::new("s-job");
    let session = session(&rig);
    run_session(&session, &plan(1, 0));
    let job = rig
        .host
        .state_dir
        .join(format!("runs/{STAMP}/r02/fwcfg/job"));
    assert_eq!(
        std::fs::read_to_string(job).unwrap(),
        "00000000000000000001"
    );
    let prep = rig
        .host
        .state_dir
        .join(format!("runs/{STAMP}/r01/fwcfg/job"));
    assert_eq!(
        std::fs::read_to_string(prep).unwrap(),
        "00000000000000000001"
    );
}

// ---- OI-5: the RDRAND gate ----------------------------------------------------------------------------------------------

#[test]
fn a_warning_in_the_prepares_standard_error_ends_the_session_unless_it_was_accepted() {
    let rig = Rig::new("s-rdrand");
    rig.modes(&["rdrand_warning"]);
    let session = session(&rig);
    let summary = run_session(&session, &plan(2, 1));
    assert_eq!(summary.end, "END:RDRAND");
    assert_eq!(summary.control.count, 0);
    let rows = step_rows(&rig, &session);
    let gr = rows.iter().find(|r| r[2] == "GR").unwrap();
    assert_eq!(gr[6], "rdrand_p=warning flag=no");
    assert_eq!(
        rig.calls()
            .iter()
            .filter(|c| c.starts_with("bhyve "))
            .count(),
        1,
        "only the prepare ran"
    );

    let rig = Rig::new("s-rdrand-ok");
    rig.modes(&["rdrand_warning"]);
    let session = session_for(&rig);
    let mut p = plan(1, 0);
    p.rdrand_accepted = true;
    let summary = run_session(&session, &p);
    assert_eq!(summary.end, "DONE");
    let gr = step_rows(&rig, &session)
        .into_iter()
        .find(|r| r[2] == "GR")
        .unwrap();
    assert_eq!(gr[6], "rdrand_p=warning flag=yes");
}

fn session_for(rig: &Rig) -> Session {
    session(rig)
}

#[test]
fn no_warning_is_recorded_as_none() {
    let rig = Rig::new("s-rdrand-none");
    let session = session(&rig);
    run_session(&session, &plan(1, 0));
    let gr = step_rows(&rig, &session)
        .into_iter()
        .find(|r| r[2] == "GR")
        .unwrap();
    assert_eq!(gr[6], "rdrand_p=none flag=no");
}

// ---- OI-6: END codes ----------------------------------------------------------------------------------------------------

#[test]
fn a_changed_checkpoint_ends_the_session_and_does_not_go_on() {
    let rig = Rig::new("s-ckpt");
    rig.modes(&["", "tamper_ckpt"]);
    let session = session(&rig);
    let summary = run_session(&session, &plan(4, 2));
    assert_eq!(summary.end, "END:CKPT-CHANGED");
    assert_eq!(
        rig.calls()
            .iter()
            .filter(|c| c.starts_with("bhyve "))
            .count(),
        2,
        "the next restore never started"
    );
}

#[test]
fn the_codes_of_the_stops_that_end_a_session_are_named_for_the_faults_that_cause_them() {
    for (kind, code) in [
        (InfraKind::CheckpointChanged, Some("END:CKPT-CHANGED")),
        (InfraKind::InputFiles, Some("END:FWCFG-MISMATCH")),
        (InfraKind::Refused, Some("END:ARGV-REFUSED")),
        (InfraKind::NoConnect, None),
        (InfraKind::ResumeNotTaken, None),
        (InfraKind::VmmChanged, None),
    ] {
        assert_eq!(end_code(&InfraError::bare(kind)), code, "{kind:?}");
    }
}

#[test]
fn a_program_that_cannot_start_is_a_prepare_failure_and_not_a_refusal() {
    let rig = Rig::new("s-refused");
    let mut host = rig.host.clone();
    host.session_stamp = Some(Stamp::parse(STAMP).unwrap());
    host.core = 2;
    host.cpuset = "/nonexistent/cpuset".into();
    let session = Session::new(host, common::quick(), common::tiny_bounds())
        .with_one_prepare()
        .with_keep_all();
    let summary = run_session(&session, &plan(1, 0));
    assert_eq!(
        summary.end, "END:PREPARE",
        "a program that cannot start is not a refusal"
    );
}

// ---- OI-7: the start rule before the prepare -----------------------------------------------------------------------------

#[test]
fn nothing_starts_if_the_prepare_cannot_end_before_the_deadline() {
    let rig = Rig::new("s-wcap-prepare");
    let session = session(&rig);
    let mut p = plan(2, 1);
    p.deadline = Some(Instant::now() + Duration::from_secs(60));
    p.prepare_worst = Duration::from_secs(445);
    let summary = run_session(&session, &p);
    assert_eq!(summary.end, "END:WCAP");
    assert!(summary.prepare.is_err());
    assert!(rig.calls().is_empty());
}

#[test]
fn a_restore_is_refused_when_what_is_left_is_less_than_its_worst_case() {
    let rig = Rig::new("s-wcap-run");
    let session = session(&rig);
    let mut p = plan(2, 1);
    p.deadline = Some(Instant::now() + Duration::from_secs(60));
    p.prepare_worst = Duration::from_secs(10);
    p.run_worst = Duration::from_secs(300);
    let summary = run_session(&session, &p);
    assert_eq!(summary.end, "END:WCAP");
    assert!(summary.prepare.is_ok());
    assert_eq!(summary.control.count, 0);
}

// ---- OI-8: what counts -----------------------------------------------------------------------------------------------------

#[test]
fn a_restore_counts_only_if_the_guest_printed_the_seed_its_arm_gives() {
    let rig = Rig::new("s-v7");
    rig.modes(&["", "wrong_seed_line"]);
    let session = session(&rig);
    let summary = run_session(&session, &plan(3, 1));
    assert_eq!(
        summary.end, "END:FIRST-NOT-OK",
        "position 1 printed another seed"
    );
    assert_eq!(summary.control.counted, 0);
    let rig = Rig::new("s-v7-missing");
    rig.modes(&["", "no_seed_line"]);
    let summary = run_session(&session_for(&rig), &plan(3, 1));
    assert_eq!(
        summary.end, "END:FIRST-NOT-OK",
        "no seed line is not the arm's seed"
    );
    let rig = Rig::new("s-v7-later");
    rig.modes(&["", "", "wrong_seed_line", ""]);
    let summary = run_session(&session_for(&rig), &plan(3, 0));
    assert_eq!(summary.end, "DONE");
    assert_eq!((summary.control.count, summary.control.counted), (3, 2));
}

#[test]
fn the_other_arm_is_counted_against_its_own_seed() {
    let rig = Rig::new("s-v7-s");
    let session = session(&rig);
    let summary = run_session(&session, &plan(2, 1));
    assert_eq!(
        (summary.control.counted, summary.other_seed.counted),
        (2, 1)
    );
}

// ---- OI-10, OI-11, OI-12, OI-13, OI-16: what a record keeps ----------------------------------------------------------------

#[test]
fn a_record_keeps_events_whole_names_its_scan_in_words_and_keeps_the_serial_and_the_state_sizes() {
    let rig = Rig::new("s-record");
    let long = format!("HAIRSPRING_EVENT {{\"v\":1,\"kind\":\"hello\",\"sdk\":\"c\",\"sdk_version\":\"0.1.0\",\"proc\":\"p1\",\"catalog_hash\":\"{}\",\"signal_handlers\":false}}", "a".repeat(110));
    assert!(long.len() > 150);
    rig.events(&[
        &long,
        r#"HAIRSPRING_EVENT {"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":0,"evals":0,"dropped":0,"threads":0}"#,
    ]);
    let session = session(&rig);
    run_session(&session, &plan(1, 0));
    let r = record(&rig, &session, &format!("vmvx_{STAMP}_XB_r02"));
    assert_eq!(
        r["events"][0].as_str().unwrap(),
        long,
        "an event is not cut"
    );
    assert_eq!(r["scan"], "seen");
    assert!(r["fwcfg_sha256"].is_object());
    assert_eq!(r["fwcfg_sha256"].as_object().unwrap().len(), 6);
    assert_eq!(r["hs_seed"], "1007");
    let suffix = r["serial_suffix"].as_array().unwrap();
    assert_eq!(suffix.last().unwrap(), "HS_EXIT 0 0");
    assert!(suffix.iter().any(|l| l == "HS_SEED 1007"));
    assert_eq!(r["state_files"]["state_mem.bin"], 16);
    assert_eq!(r["state_files"].as_object().unwrap().len(), 4);
}

#[test]
fn the_serial_and_event_digests_leave_out_the_seed_and_long_line_lines() {
    use hairspring_dhyve::judge::digest_lines;
    let a: Vec<Vec<u8>> = ["HS_SEED 1007", "line", "HS_LONG 4095"]
        .iter()
        .map(|s| s.as_bytes().to_vec())
        .collect();
    let b: Vec<Vec<u8>> = ["HS_SEED 1008", "line", "HS_LONG 1"]
        .iter()
        .map(|s| s.as_bytes().to_vec())
        .collect();
    let c: Vec<Vec<u8>> = ["line", "other"]
        .iter()
        .map(|s| s.as_bytes().to_vec())
        .collect();
    assert_eq!(digest_lines(&a, &|_| true), digest_lines(&b, &|_| true));
    assert_ne!(digest_lines(&a, &|_| true), digest_lines(&c, &|_| true));
}
