//! An interrupt (Ctrl-C in the CLI) stops the guest of a running trial and everything the guest started, through the
//! same process-group kill that ends a trial at its timeout. This file holds one test because the interrupt is
//! process-wide: it would stop the trials of any other test running beside it.
#![cfg(unix)]
mod common;
use common::events::rig_with;
use hairspring_hermit::interrupt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

const PATIENCE: Duration = Duration::from_secs(10);

/// A guest that records its own pid and that of a background child, then waits for the child.
const GUEST: &str =
    "echo $$ > \"$EV_LOG/guest.pid\"; sleep 60 & echo $! > \"$EV_LOG/child.pid\"; wait";

fn wait_for_pid(file: &Path) -> String {
    let until = Instant::now() + PATIENCE;
    while Instant::now() < until {
        if let Ok(text) = std::fs::read_to_string(file) {
            if !text.trim().is_empty() {
                return text.trim().to_string();
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{} never appeared", file.display());
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .output()
        .unwrap()
        .status
        .success()
}

fn gone_soon(pid: &str) -> bool {
    let until = Instant::now() + PATIENCE;
    while Instant::now() < until {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn an_interrupt_kills_the_guest_and_its_children_and_the_trial_reports_it() {
    let rig = rig_with("interrupt", GUEST, false, vec![], |_| {});
    let (guest_file, child_file) = (rig.dir.join("guest.pid"), rig.dir.join("child.pid"));
    let started = Instant::now();
    let result = std::thread::scope(|scope| {
        let trial = scope.spawn(|| rig.backend.run_one(&rig.spec, 1));
        let (guest, child) = (wait_for_pid(&guest_file), wait_for_pid(&child_file));
        assert!(
            alive(&guest) && alive(&child),
            "both run before the interrupt"
        );
        interrupt::request();
        (trial.join().unwrap(), guest, child)
    });
    let (outcome, guest, child) = result;
    let err = outcome
        .expect_err("an interrupted trial is not a result")
        .to_string();
    assert!(err.contains("interrupted"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "it did not wait out the guest"
    );
    assert!(gone_soon(&guest), "the guest is still running");
    assert!(gone_soon(&child), "the guest's child is still running");

    // Once the flag is cleared, trials run again.
    interrupt::clear();
    let after = rig_with("after", "echo ok", false, vec![], |_| {});
    assert!(after.backend.run_one(&after.spec, 2).is_ok());
}
