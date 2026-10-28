//! Ctrl-C on `hairspring test` and `hairspring confirm`: the CLI stops, the Hermit guests it started are killed with
//! their process groups, the run is left cancelled and the exit status is 130.
#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PATIENCE: Duration = Duration::from_secs(30);
const EXIT_INTERRUPTED: i32 = 130;
const FAKE_HERMIT: &str = "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 'fake-hermit 0.0'; exit 0; }\nshift\nseed=0\nwhile [ \"$1\" != \"--\" ]; do case \"$1\" in --sched-seed=*) seed=${1#--sched-seed=};; esac; shift; done\nshift\nFAKE_SCHED_SEED=$seed exec \"$@\"\n";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-interrupt-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A guest that records its pid and that of a background child in `dir`, then waits for the child.
fn hanging(dir: &Path) -> String {
    format!(
        "echo $$ > {d}/guest.pid; sleep 60 & echo $! > {d}/child.pid; wait",
        d = dir.display()
    )
}

fn project(dir: &Path, guest: &str) {
    use std::os::unix::fs::PermissionsExt;
    let hermit = dir.join("fake-hermit");
    std::fs::write(&hermit, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&hermit, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = format!(
        "[target.svc]\nkind = \"process\"\ncommand = [\"/bin/sh\", \"-c\", {guest:?}]\n[target.svc.budget]\nmax_trials = 20\n"
    );
    std::fs::write(dir.join("hairspring.toml"), config).unwrap();
}

fn spawn(dir: &Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(args)
        .current_dir(dir)
        .env("HAIRSPRING_HERMIT", dir.join("fake-hermit"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary runs")
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    spawn(dir, args).wait_with_output().unwrap()
}

fn text(o: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn wait_for(file: &Path) -> String {
    let until = Instant::now() + PATIENCE;
    while Instant::now() < until {
        if let Ok(t) = std::fs::read_to_string(file) {
            if !t.trim().is_empty() {
                return t.trim().to_string();
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

fn gone(pid: &str) -> bool {
    let until = Instant::now() + PATIENCE;
    while Instant::now() < until {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Sends SIGINT to the CLI once its guest is running, and returns the guest's and its child's pids and the CLI's output.
fn interrupt_once_running(dir: &Path, mut cli: Child) -> (String, String, std::process::Output) {
    let guest = wait_for(&dir.join("guest.pid"));
    let child = wait_for(&dir.join("child.pid"));
    assert!(alive(&guest) && alive(&child));
    let sent = Command::new("kill")
        .args(["-INT", &cli.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    let until = Instant::now() + PATIENCE;
    while cli.try_wait().unwrap().is_none() {
        assert!(Instant::now() < until, "the CLI did not stop after Ctrl-C");
        std::thread::sleep(Duration::from_millis(20));
    }
    (guest, child, cli.wait_with_output().unwrap())
}

fn latest_run_state(dir: &Path) -> String {
    let listing = run(dir, &["results", "--limit", "1"]);
    text(&listing)
        .split_whitespace()
        .nth(1)
        .expect("a state")
        .to_string()
}

#[test]
fn ctrl_c_during_a_sweep_kills_the_guests_cancels_the_run_and_exits_130() {
    let dir = scratch("sweep");
    project(&dir, &hanging(&dir));
    let cli = spawn(
        &dir,
        &["test", "svc", "--sweep", "--trials", "5", "--seed", "1"],
    );
    let (guest, child, out) = interrupt_once_running(&dir, cli);
    assert_eq!(out.status.code(), Some(EXIT_INTERRUPTED), "{}", text(&out));
    let said = text(&out);
    assert!(said.contains("interrupted"), "{said}");
    assert!(said.contains("cancelled"), "{said}");
    assert!(gone(&guest), "the guest is still running");
    assert!(gone(&child), "the guest's child is still running");
    assert_eq!(latest_run_state(&dir), "cancelled");
}

#[test]
fn ctrl_c_during_a_search_run_exits_130_and_leaves_no_guest_running() {
    let dir = scratch("search");
    project(&dir, &hanging(&dir));
    let cli = spawn(&dir, &["test", "svc", "--seed", "1"]);
    let (guest, child, out) = interrupt_once_running(&dir, cli);
    assert_eq!(out.status.code(), Some(EXIT_INTERRUPTED), "{}", text(&out));
    assert!(gone(&guest) && gone(&child));
    assert_eq!(latest_run_state(&dir), "cancelled");
}

#[test]
fn ctrl_c_during_a_confirmation_kills_the_reruns_and_exits_130() {
    let dir = scratch("confirm");
    // Seed 3 fails; once the HANG file exists a run of it hangs instead, as a rerun that never finishes would.
    let guest = format!(
        "[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] || {{ echo ok; exit 0; }}; [ -f {d}/HANG ] && {{ {hang}; }}; echo 'HAIRSPRING_VIOLATION: lost update'; exit 1",
        d = dir.display(),
        hang = hanging(&dir)
    );
    project(&dir, &guest);
    let first = run(&dir, &["test", "svc", "--seed", "1"]);
    assert_eq!(first.status.code(), Some(1), "{}", text(&first));
    let run_id = text(&run(&dir, &["results", "--limit", "1"]))
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    std::fs::write(dir.join("HANG"), "").unwrap();
    let cli = spawn(&dir, &["confirm", &run_id]);
    let (guest, child, out) = interrupt_once_running(&dir, cli);
    assert_eq!(out.status.code(), Some(EXIT_INTERRUPTED), "{}", text(&out));
    assert!(text(&out).contains("interrupted"), "{}", text(&out));
    assert!(gone(&guest) && gone(&child));
}

/// The lines of `hairspring show` that state what the stored checks say about the failure.
fn finding_lines(dir: &Path, run_id: &str) -> Vec<String> {
    text(&run(dir, &["show", run_id]))
        .lines()
        .filter(|l| l.contains("finding"))
        // The number of checks grows with every confirmation; the standing is what is compared.
        .map(|l| l.split(" (").next().unwrap_or(l).to_string())
        .collect()
}

#[test]
fn ctrl_c_during_the_bundle_replay_stores_it_as_skipped_and_leaves_the_earlier_finding_alone() {
    let dir = scratch("replay");
    // Seed 3 fails. Once ARMED exists, the fourth run of it hangs: the three reruns come first, so that is the replay.
    let guest = format!(
        "[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] || {{ echo ok; exit 0; }}; if [ -f {d}/ARMED ]; then n=$(cat {d}/count 2>/dev/null || echo 0); n=$((n+1)); echo $n > {d}/count; [ $n -ge 4 ] && {{ {hang}; }}; fi; echo 'HAIRSPRING_VIOLATION: lost update'; exit 1",
        d = dir.display(),
        hang = hanging(&dir)
    );
    project(&dir, &guest);
    assert_eq!(
        run(&dir, &["test", "svc", "--seed", "1"]).status.code(),
        Some(1)
    );
    let run_id = text(&run(&dir, &["results", "--limit", "1"]))
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let first = run(&dir, &["confirm", &run_id]);
    assert!(
        text(&first).contains("finding: confirmed"),
        "{}",
        text(&first)
    );
    let before = finding_lines(&dir, &run_id);
    assert!(before.iter().any(|l| l.contains("confirmed")), "{before:?}");

    std::fs::write(dir.join("ARMED"), "").unwrap();
    let cli = spawn(&dir, &["confirm", &run_id]);
    let (guest_pid, child, out) = interrupt_once_running(&dir, cli);
    assert_eq!(out.status.code(), Some(EXIT_INTERRUPTED), "{}", text(&out));
    assert!(gone(&guest_pid) && gone(&child));

    assert_eq!(
        finding_lines(&dir, &run_id),
        before,
        "the standing did not change"
    );
    // The check that was running is stored as skipped for cancellation, not as one that could not run.
    use hairspring_store::{SqliteStore, Store};
    let store = SqliteStore::open(&dir.join(".hairspring/hairspring.db")).unwrap();
    let failure = store.list_failures(&run_id).unwrap().remove(0);
    let checks = store.list_failure_checks(&failure.id).unwrap();
    let newest = checks.last().expect("a stored check");
    assert_eq!(newest.kind, "bundle_replay");
    assert_eq!(newest.status, "skipped", "{newest:?}");
    assert!(newest.detail_json.contains("cancelled"), "{newest:?}");
    assert!(checks
        .iter()
        .all(|c| !c.detail_json.contains("host or Hermit fault")));
}

#[test]
fn ctrl_c_during_a_sweep_keeps_the_trials_judged_before_it() {
    let dir = scratch("kept");
    // Seed 1 passes at once; the later seeds hang.
    let guest = format!(
        "[ \"$FAKE_SCHED_SEED\" = 1 ] && {{ echo ok; exit 0; }}; {hang}",
        hang = hanging(&dir)
    );
    project(&dir, &guest);
    let cli = spawn(
        &dir,
        &["test", "svc", "--sweep", "--trials", "5", "--seed", "1"],
    );
    let (guest_pid, child, out) = interrupt_once_running(&dir, cli);
    assert_eq!(out.status.code(), Some(EXIT_INTERRUPTED), "{}", text(&out));
    assert!(gone(&guest_pid) && gone(&child));
    assert_eq!(latest_run_state(&dir), "cancelled");
    use hairspring_store::{SqliteStore, Store};
    let store = SqliteStore::open(&dir.join(".hairspring/hairspring.db")).unwrap();
    let run_id = store.list_runs(1).unwrap().remove(0).id;
    assert_eq!(
        store.count_trials(&run_id).unwrap(),
        1,
        "the passing trial is kept"
    );
}

#[test]
fn ctrl_c_while_running_another_runs_expired_job_gives_it_back_and_cancels_only_our_run() {
    use hairspring_store::{now_ms, SqliteStore, Store};
    let dir = scratch("foreign");
    project(&dir, &hanging(&dir));
    // Run A is killed hard mid-trial, leaving its job leased; its guests are cleaned up by hand.
    let mut a = spawn(
        &dir,
        &["test", "svc", "--sweep", "--trials", "5", "--seed", "1"],
    );
    let (guest, child) = (
        wait_for(&dir.join("guest.pid")),
        wait_for(&dir.join("child.pid")),
    );
    a.kill().unwrap();
    a.wait().unwrap();
    for pid in [&guest, &child] {
        let _ = Command::new("kill").args(["-9", pid]).status();
    }
    assert!(gone(&guest) && gone(&child));
    std::fs::remove_file(dir.join("guest.pid")).unwrap();
    std::fs::remove_file(dir.join("child.pid")).unwrap();
    // Let its lease run out: take the job over and give the lease back as already expired.
    let store = SqliteStore::open(&dir.join(".hairspring/hairspring.db")).unwrap();
    let a_run = store.list_runs(1).unwrap().remove(0).id;
    let taken = store
        .lease_job("probe", 0, i64::MAX / 4)
        .unwrap()
        .expect("A's job");
    assert_eq!(taken.run_id, a_run);
    assert!(store.heartbeat_job(&taken.id, "probe", 0, 0).unwrap());
    drop(store);

    // Run B leases A's job first. Ctrl-C while A's trial runs.
    let b = spawn(
        &dir,
        &["test", "svc", "--sweep", "--trials", "5", "--seed", "1"],
    );
    let (guest, child, out) = interrupt_once_running(&dir, b);
    assert_eq!(out.status.code(), Some(EXIT_INTERRUPTED), "{}", text(&out));
    assert!(gone(&guest) && gone(&child));

    let store = SqliteStore::open(&dir.join(".hairspring/hairspring.db")).unwrap();
    let a_after = store.get_run(&a_run).unwrap().unwrap();
    assert!(
        !a_after.state.is_terminal(),
        "A is not finished or failed: {a_after:?}"
    );
    let b_run = store
        .list_runs(10)
        .unwrap()
        .into_iter()
        .find(|r| r.id != a_run)
        .unwrap();
    assert_eq!(b_run.state.as_str(), "cancelled");
    let again = store.lease_job("probe-2", 30, now_ms()).unwrap();
    assert_eq!(
        again.map(|j| j.run_id),
        Some(a_run),
        "A's job is queued and can be leased again"
    );
}
