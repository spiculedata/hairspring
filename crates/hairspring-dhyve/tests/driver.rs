//! The session driver, as a binary and in process, against the fake VM programs: the literal order, the names, the seeds,
//! the stop after a first restore that is not counted, the start rule, the refusal of a second prepare, the counts of the
//! root commands, and the two private records.
mod common;
use common::Rig;
use hairspring_core::environment::sha256_file;
use hairspring_dhyve::driver::{order, records_dir, run_session, steps_log, Plan};
use hairspring_dhyve::host::{
    ENV_BHYVE, ENV_BHYVECTL, ENV_INITRD, ENV_KERNEL, ENV_LIB, ENV_PIN_BHYVE, ENV_PIN_BHYVECTL,
    ENV_PIN_LIB, ENV_STAMP, ENV_STATE, ENV_SUDO,
};
use hairspring_dhyve::pins::lib_manifest_pin;
use serde_json::Value;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

const DRIVER: &str = env!("CARGO_BIN_EXE_dhyve-session-driver");
const STAMP: &str = "20261201T0540Z";

fn drive(rig: &Rig, args: &[&str]) -> Output {
    let mut with_flag = args.to_vec();
    with_flag.push("--allow-fake");
    drive_without(rig, &with_flag, &[])
}

/// The driver with every setting a session needs, minus the variables named in `omit`.
fn drive_without(rig: &Rig, args: &[&str], omit: &[&str]) -> Output {
    let pin = |p: &std::path::Path| sha256_file(p).unwrap();
    let vars: Vec<(&str, String)> = vec![
        (ENV_SUDO, rig.host.sudo.display().to_string()),
        (
            "HAIRSPRING_DHYVE_KLDSTAT",
            rig.host.kldstat.display().to_string(),
        ),
        (
            "HAIRSPRING_DHYVE_CPUSET",
            rig.host.cpuset.display().to_string(),
        ),
        (ENV_BHYVE, rig.host.bhyve.display().to_string()),
        (ENV_BHYVECTL, rig.host.bhyvectl.display().to_string()),
        (ENV_LIB, rig.host.lib_dir.display().to_string()),
        (ENV_STATE, rig.host.state_dir.display().to_string()),
        (ENV_KERNEL, rig.host.kernel.display().to_string()),
        (ENV_INITRD, rig.host.initrd.display().to_string()),
        (ENV_STAMP, STAMP.to_string()),
        (ENV_PIN_BHYVE, pin(&rig.host.bhyve)),
        (ENV_PIN_BHYVECTL, pin(&rig.host.bhyvectl)),
        (ENV_PIN_LIB, lib_manifest_pin(&rig.host.lib_dir).unwrap()),
    ];
    let mut command = Command::new(DRIVER);
    command.args(args).env_clear();
    for (name, value) in vars.iter().filter(|(n, _)| !omit.contains(n)) {
        command.env(name, value);
    }
    command.output().unwrap()
}

fn summary(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn plan(control: u32, other: u32) -> Plan {
    Plan {
        seed: 1007,
        control_restores: control,
        other_seed_restores: other,
        deadline: None,
    }
}

fn session(rig: &Rig) -> hairspring_dhyve::session::Session {
    let mut host = rig.host.clone();
    host.session_stamp = Some(hairspring_dhyve::vmname::Stamp::parse(STAMP).unwrap());
    hairspring_dhyve::session::Session::new(host, common::quick(), common::tiny_bounds())
        .with_one_prepare()
        .with_keep_all()
}

fn starts_and_destroys(rig: &Rig) -> (usize, usize) {
    let calls = rig.calls();
    (
        calls.iter().filter(|c| c.starts_with("bhyve ")).count(),
        calls.iter().filter(|c| c.contains("--destroy")).count(),
    )
}

#[test]
fn ten_and_five_make_the_literal_order() {
    assert_eq!(order(10, 5), "CCSCCSCCSCCSCCS");
    assert_eq!(order(3, 1), "CCSC");
    assert_eq!(order(1, 2), "CSS");
    assert_eq!(order(0, 0), "");
}

#[test]
fn the_driver_binary_runs_the_literal_order_under_the_session_stamp_and_prints_one_summary() {
    let rig = Rig::new("d-bin");
    let out = drive(&rig, &["--control", "4", "--other", "2"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = summary(&out);
    assert_eq!(s["prepare"]["ok"], true);
    assert_eq!(s["prepare"]["stamp"], STAMP);
    assert_eq!(s["end"], "DONE");
    assert_eq!(s["control"]["count"], 4);
    assert_eq!(s["other_seed"]["count"], 2);
    assert_eq!(s["control"]["resume_seen"], 4);
    assert_eq!(s["control"]["serial"], "equal");
    let names: Vec<&str> = s["names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    let expected: Vec<String> = (2..=7)
        .map(|n| format!("vmvx_{STAMP}_XB_r{n:02}"))
        .collect();
    assert_eq!(names, expected);
    let seeds: Vec<String> = rig
        .trials()
        .iter()
        .map(|l| l.split(' ').next().unwrap().to_string())
        .collect();
    assert_eq!(
        seeds,
        [
            "seed=1007",
            "seed=1007",
            "seed=1008",
            "seed=1007",
            "seed=1007",
            "seed=1008"
        ]
    );
    let text = String::from_utf8_lossy(&out.stdout);
    for word in ["deterministic", "reproduced"] {
        assert!(!text.contains(word), "{text}");
    }
}

#[test]
fn the_driver_binary_exits_1_after_an_error_and_2_for_a_bad_command_line() {
    let rig = Rig::new("d-bin-bad");
    rig.modes(&["", "", "stale_stop", ""]);
    let out = drive(&rig, &["--control", "3", "--other", "0"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        summary(&out)["control"]["errors"].as_array().unwrap().len(),
        1
    );
    for args in [
        vec!["--control", "x", "--other", "1"],
        vec!["--other", "1"],
        vec!["--control", "1"],
        vec!["--control", "1", "--other", "1", "--bogus", "1"],
    ] {
        assert_eq!(drive(&rig, &args).status.code(), Some(2), "{args:?}");
    }
    let none = Command::new(DRIVER)
        .args(["--control", "1", "--other", "1", "--allow-fake"])
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(none.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&none.stderr).contains(ENV_BHYVE));
}

#[test]
fn a_prepare_that_fails_is_reported_and_no_restore_runs() {
    let rig = Rig::new("d-prep");
    rig.modes(&["drop_before_ready"]);
    let out = drive(&rig, &["--control", "2", "--other", "1"]);
    assert_eq!(out.status.code(), Some(1));
    let s = summary(&out);
    assert_eq!(s["prepare"]["ok"], false);
    assert_eq!(s["control"]["count"], 0);
    assert_eq!(s["end"], "END:PREPARE");
}

#[test]
fn the_session_runs_c_c_s_with_names_r02_to_r16_and_two_destroys_for_each_start() {
    let rig = Rig::new("d-lib");
    rig.events(&["line {seed}"]);
    let summary = run_session(&session(&rig), &plan(10, 5));
    assert!(summary.clean(), "{}", summary.to_json());
    assert_eq!(summary.names.len(), 15);
    assert_eq!(summary.names[0], format!("vmvx_{STAMP}_XB_r02"));
    assert_eq!(summary.names[14], format!("vmvx_{STAMP}_XB_r16"));
    let (starts, destroys) = starts_and_destroys(&rig);
    assert_eq!(starts, 16, "one prepare and fifteen restores");
    assert!(
        destroys <= 2 * starts,
        "{destroys} destroys for {starts} starts"
    );
    let seeds: Vec<String> = rig
        .trials()
        .iter()
        .map(|l| l.split(' ').next().unwrap().to_string())
        .collect();
    let literal = "CCSCCSCCSCCSCCS";
    for (seed, group) in seeds.iter().zip(literal.chars()) {
        assert_eq!(
            seed,
            if group == 'C' {
                "seed=1007"
            } else {
                "seed=1008"
            }
        );
    }
    let json: Value = serde_json::from_str(&summary.to_json()).unwrap();
    assert_eq!(
        json["control"]["serial"], "equal",
        "the C runs all read the same seed file"
    );
}

#[test]
fn the_session_stops_when_the_first_restore_is_not_counted_and_not_when_a_later_one_is_not() {
    let rig = Rig::new("d-first");
    rig.modes(&["", "stale_stop"]);
    let summary = run_session(&session(&rig), &plan(4, 2));
    assert_eq!(summary.end, "END:FIRST-NOT-OK");
    assert_eq!(
        starts_and_destroys(&rig).0,
        2,
        "the prepare and one restore"
    );
    let rig = Rig::new("d-later");
    rig.modes(&["", "", "stale_stop", ""]);
    let summary = run_session(&session(&rig), &plan(4, 2));
    assert_eq!(summary.end, "DONE");
    assert_eq!(
        summary.control.errors.len(),
        1,
        "position 2 is the second C"
    );
    assert_eq!(summary.other_seed.errors.len(), 0);
    assert_eq!(starts_and_destroys(&rig).0, 7, "every position still ran");
}

#[test]
fn a_restore_starts_only_if_it_can_end_before_the_deadline() {
    let rig = Rig::new("d-wcap");
    let late = Plan {
        deadline: Some(Instant::now() + Duration::from_secs(10)),
        ..plan(4, 2)
    };
    let summary = run_session(&session(&rig), &late);
    assert_eq!(
        summary.end, "END:WCAP",
        "ten seconds is less than the worst of one restore"
    );
    assert!(summary.prepare.is_ok());
    assert_eq!(summary.control.count + summary.other_seed.count, 0);
    assert!(!summary.clean());
}

#[test]
fn a_second_session_on_the_same_prepared_session_is_refused() {
    let rig = Rig::new("d-twice");
    let session = session(&rig);
    assert!(run_session(&session, &plan(1, 0)).clean());
    let starts = starts_and_destroys(&rig).0;
    let again = run_session(&session, &plan(1, 0));
    assert_eq!(again.end, "END:REPREPARE");
    assert_eq!(starts_and_destroys(&rig).0, starts, "nothing was started");
}

#[test]
fn the_steps_log_and_a_record_for_each_run_are_written_and_the_prepares_log_is_kept() {
    let rig = Rig::new("d-records");
    rig.events(&[
        r#"HAIRSPRING_EVENT {"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"abc","signal_handlers":false}"#,
        r#"HAIRSPRING_EVENT {"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":0,"evals":0,"dropped":0,"threads":0}"#,
        "plain",
    ]);
    let session = session(&rig);
    let summary = run_session(&session, &plan(2, 1));
    assert!(summary.clean(), "{}", summary.to_json());
    let dir = records_dir(&session);
    let steps = std::fs::read_to_string(steps_log(&dir)).unwrap();
    let rows: Vec<Vec<&str>> = steps.lines().map(|l| l.split('\t').collect()).collect();
    assert!(
        rows.iter().all(|r| r.len() == 5 && r[1] == "driver"),
        "{steps}"
    );
    assert!(rows.iter().any(|r| r[2] == "prepare" && r[4] == "ok"));
    assert_eq!(
        rows.iter()
            .filter(|r| r[2] == "trial" && r[4] == "pass")
            .count(),
        3
    );
    assert_eq!(rows.last().unwrap()[2], "end");
    let record: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(format!("vmvx_{STAMP}_XB_r04.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(record["group"], "S");
    assert_eq!(record["position"], 3);
    assert_eq!(record["verdict"], "pass");
    assert_eq!(record["hs_exit"]["code"], 0);
    assert_eq!(record["fwcfg_sha256"].as_array().unwrap().len(), 6);
    assert_eq!(record["restore_keys"]["stop_seq"], 1);
    assert_eq!(record["restored_pointer"], "0xffffffff80123456");
    assert_eq!(record["argv_masked_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(
        record["events"].as_array().unwrap().len(),
        2,
        "the event lines are stored"
    );
    let prepare_log = rig.host.state_dir.join(format!("runs/{STAMP}/r01/run.log"));
    assert!(
        prepare_log.exists(),
        "the prepare's standard error is kept in full"
    );
}

#[test]
fn the_driver_refuses_to_start_without_the_stamp_or_any_of_the_three_pins() {
    let rig = Rig::new("d-required");
    for missing in [ENV_STAMP, ENV_PIN_BHYVE, ENV_PIN_BHYVECTL, ENV_PIN_LIB] {
        let out = drive_without(
            &rig,
            &["--control", "1", "--other", "0", "--allow-fake"],
            &[missing],
        );
        assert_eq!(out.status.code(), Some(2), "{missing}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(missing),
            "{missing}"
        );
    }
    assert!(
        rig.calls().is_empty(),
        "nothing was started: {:?}",
        rig.calls()
    );
    let all = drive_without(
        &rig,
        &["--control", "1", "--other", "0", "--allow-fake"],
        &[],
    );
    assert_eq!(
        all.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&all.stderr)
    );
}

#[test]
fn a_build_with_the_stand_in_programs_needs_a_flag_and_says_so_in_its_summary() {
    let rig = Rig::new("d-fake-flag");
    let out = drive_without(&rig, &["--control", "1", "--other", "0"], &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("stand-in"));
    assert!(rig.calls().is_empty());
    let out = drive(&rig, &["--control", "1", "--other", "0"]);
    assert_eq!(summary(&out)["fake_vmm"], true);
}

#[test]
fn the_build_info_flag_says_whether_the_stand_in_programs_are_built_in_and_runs_nothing() {
    let rig = Rig::new("d-info");
    let out = drive_without(&rig, &["--build-info"], &[ENV_STAMP, ENV_PIN_BHYVE]);
    assert_eq!(out.status.code(), Some(0));
    let info: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(info["fake_vmm"], true, "this test build has the feature on");
    assert!(rig.calls().is_empty());
}

#[test]
fn the_deadline_is_seconds_from_the_drivers_own_start_so_the_executor_hands_in_what_is_left() {
    let rig = Rig::new("d-deadline");
    let out = drive(
        &rig,
        &["--control", "2", "--other", "1", "--deadline-secs", "5"],
    );
    assert_eq!(out.status.code(), Some(1));
    let s = summary(&out);
    assert_eq!(
        s["end"], "END:WCAP",
        "five seconds is less than one run's worst case"
    );
    assert_eq!(s["control"]["count"], 0);
}
