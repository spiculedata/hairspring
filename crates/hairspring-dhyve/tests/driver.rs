//! The session driver binary against the fake VM programs.
mod common;
use common::Rig;
use hairspring_dhyve::host::{
    ENV_BHYVE, ENV_BHYVECTL, ENV_INITRD, ENV_KERNEL, ENV_LIB, ENV_STATE, ENV_SUDO,
};
use serde_json::Value;
use std::process::{Command, Output};

const DRIVER: &str = env!("CARGO_BIN_EXE_dhyve-session-driver");

fn drive(rig: &Rig, args: &[&str]) -> Output {
    Command::new(DRIVER)
        .args(args)
        .env_clear()
        .env(ENV_SUDO, &rig.host.sudo)
        .env(ENV_BHYVE, &rig.host.bhyve)
        .env(ENV_BHYVECTL, &rig.host.bhyvectl)
        .env(ENV_LIB, &rig.host.lib_dir)
        .env(ENV_STATE, &rig.host.state_dir)
        .env(ENV_KERNEL, &rig.host.kernel)
        .env(ENV_INITRD, &rig.host.initrd)
        .output()
        .unwrap()
}

fn summary(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)))
}

#[test]
fn the_driver_prepares_runs_both_groups_and_prints_one_json_summary() {
    let rig = Rig::new("d-ok");
    let out = drive(&rig, &["--control", "3", "--other", "2", "--seed", "40"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{} {}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let s = summary(&out);
    assert_eq!(s["prepare"]["ok"], true);
    assert_eq!(s["prepare"]["restored_pointer"], "0xffffffff80123456");
    assert_eq!(
        s["prepare"]["checkpoint_sha256"].as_array().unwrap().len(),
        3
    );
    for (group, n) in [("control", 3), ("other_seed", 2)] {
        assert_eq!(s[group]["count"], n, "{group}");
        assert_eq!(s[group]["resume_seen"], n, "{group}");
        assert_eq!(s[group]["exit_reported"], n, "{group}");
        assert_eq!(s[group]["errors"].as_array().unwrap().len(), 0, "{group}");
    }
    assert_eq!(s["control"]["serial"], "equal");
    assert_eq!(
        s["control"]["events"], "not_compared",
        "the guest printed no event lines"
    );
    assert!(
        s["other_seed"].get("serial").is_none(),
        "no comparison across seeds"
    );
    let seeds = rig.trials();
    assert_eq!(seeds.len(), 5);
    assert!(
        seeds[..3].iter().all(|l| l.starts_with("seed=40 ")),
        "{seeds:?}"
    );
    assert!(
        seeds[3].starts_with("seed=41 ") && seeds[4].starts_with("seed=42 "),
        "{seeds:?}"
    );
    let text = String::from_utf8_lossy(&out.stdout);
    for word in ["deterministic", "reproduced"] {
        assert!(!text.contains(word), "{text}");
    }
}

#[test]
fn a_restore_that_meets_an_infrastructure_error_is_counted_and_the_exit_status_says_so() {
    let rig = Rig::new("d-infra");
    rig.modes(&["", "", "stale_stop", ""]);
    let out = drive(&rig, &["--control", "3", "--other", "0"]);
    assert_eq!(out.status.code(), Some(1));
    let s = summary(&out);
    assert_eq!(s["control"]["count"], 3);
    assert_eq!(s["control"]["resume_seen"], 2);
    assert_eq!(s["control"]["errors"].as_array().unwrap().len(), 1);
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
}

#[test]
fn a_bad_command_line_or_a_missing_setting_exits_2_and_starts_nothing() {
    let rig = Rig::new("d-bad");
    for args in [
        vec!["--control", "x", "--other", "1"],
        vec!["--other", "1"],
        vec!["--control", "1"],
        vec!["--control", "1", "--other", "1", "--bogus", "1"],
    ] {
        assert_eq!(drive(&rig, &args).status.code(), Some(2), "{args:?}");
    }
    let no_settings = Command::new(DRIVER)
        .args(["--control", "1", "--other", "1"])
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(no_settings.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&no_settings.stderr).contains(ENV_BHYVE));
    assert!(rig.calls().is_empty());
}

#[test]
fn the_session_logic_counts_in_process_and_labels_differences_among_the_control_restores() {
    use hairspring_dhyve::driver::{run_session, Plan};
    let rig = Rig::new("d-lib");
    rig.modes(&["", "", ""]);
    rig.events(&["line {seed}"]);
    let plan = Plan {
        seed: 5,
        control_restores: 2,
        other_seed_restores: 1,
    };
    let summary = run_session(&rig.session(), &plan);
    assert!(summary.clean(), "{}", summary.to_json());
    assert_eq!(summary.control.resume_seen, 2);
    assert_eq!(summary.other_seed.exit_reported, 1);
    let json: Value = serde_json::from_str(&summary.to_json()).unwrap();
    assert_eq!(json["control"]["serial"], "equal");
    let failing = Rig::new("d-lib-bad");
    failing.modes(&["no_connect"]);
    let bad = run_session(&failing.session(), &plan);
    assert!(!bad.clean() && bad.prepare.is_err());
    assert_eq!(bad.control.count, 0);
}
