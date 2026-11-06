//! `hairspring serve --max-confirmations`: confirming a stored failure from the API, with the real binary. A process
//! target runs under a fake Hermit (the real one needs Linux on x86-64); a simulation target is refused.
#![cfg(unix)]
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// The Hermit every test uses; it only waits while a file named `sleep` is beside it, so it is the same file (and the
/// same recorded identity) before and after a test makes it slow.
const FAKE_HERMIT: &str = SLOW_HERMIT;
const PROJECT: &str = "[target.svc]\nkind = \"process\"\ncommand = [\"/bin/sh\", \"-c\", \"[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: lost update'; exit 1; }; echo ok\"]\n[target.svc.budget]\nmax_trials = 20\n";
const RUN: &str = "run-token-0123456789abcdef";
const READ: &str = "read-token-0123456789abcdef";

struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("hairspring-srvconf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cli(dir: &Path, hermit: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(args)
        .current_dir(dir)
        .env("HAIRSPRING_HERMIT", hermit)
        .output()
        .expect("binary runs")
}

/// A project directory whose process target `svc` fails at seed 3, with one stored failure of it. Returns the
/// directory and the fake Hermit.
fn project(name: &str) -> (PathBuf, PathBuf) {
    let dir = scratch(name);
    let hermit = dir.join("fake-hermit");
    std::fs::write(&hermit, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&hermit, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("hairspring.toml"), PROJECT).unwrap();
    let test = cli(&dir, &hermit, &["test", "svc", "--seed", "1"]);
    assert_eq!(
        test.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&test.stdout)
    );
    (dir, hermit)
}

fn serve(
    dir: &Path,
    hermit: &Path,
    extra: &[&str],
    tokens: &str,
) -> (Guard, String, Receiver<String>) {
    let mut args = vec!["serve", "--addr", "127.0.0.1:0", "--workers", "0"];
    args.extend_from_slice(extra);
    let mut child = Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(&args)
        .current_dir(dir)
        .env("HAIRSPRING_HERMIT", hermit)
        .env("HAIRSPRING_TOKENS", tokens)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let stderr = child.stderr.take().expect("stderr piped");
    let (tx, errors) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let stdout = child.stdout.take().expect("stdout piped");
    let mut lines = BufReader::new(stdout).lines();
    let base = loop {
        let line = lines
            .next()
            .expect("the server prints a line")
            .expect("readable");
        if let Some(url) = line.split_whitespace().find(|w| w.starts_with("http://")) {
            break url.to_string();
        }
    };
    std::thread::spawn(move || for _ in lines {});
    (Guard(child), base, errors)
}

fn http(method: &str, url: &str, token: &str) -> (u16, String) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let auth = format!("Bearer {token}");
    let mut resp = if method == "GET" {
        agent.get(url).header("Authorization", auth).call()
    } else {
        agent.post(url).header("Authorization", auth).send_empty()
    }
    .expect("http ok");
    (
        resp.status().as_u16(),
        resp.body_mut().read_to_string().unwrap_or_default(),
    )
}

/// The id of the one failure the server's store holds.
fn the_failure(base: &str) -> String {
    let (status, body) = http("GET", &format!("{base}/api/failures"), READ);
    assert_eq!(status, 200, "{body}");
    body.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .find(|w| w.starts_with("fail_"))
        .unwrap_or_else(|| panic!("a failure id in: {body}"))
        .to_string()
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}

fn wait_idle(base: &str, failure: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (_, body) = http(
            "GET",
            &format!("{base}/api/failures/{failure}/confirm"),
            READ,
        );
        if json(&body)["state"] != "running" {
            return;
        }
        assert!(Instant::now() < deadline, "the confirmation did not finish");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn tokens() -> String {
    format!("{READ}:read,{RUN}:run ci bot")
}

#[test]
fn a_confirmation_started_from_the_api_is_stored_like_the_cli_and_logged_with_the_label() {
    let (dir, hermit) = project("e2e");
    let (_server, base, errors) = serve(&dir, &hermit, &["--max-confirmations", "1"], &tokens());
    let failure = the_failure(&base);

    let me = json(&http("GET", &format!("{base}/api/me"), RUN).1);
    assert_eq!(me["can"]["confirm"], true);
    let (status, body) = http(
        "POST",
        &format!("{base}/api/failures/{failure}/confirm"),
        RUN,
    );
    assert_eq!(status, 202, "{body}");
    wait_idle(&base, &failure);

    let (_, checks) = http(
        "GET",
        &format!("{base}/api/failures/{failure}/checks"),
        READ,
    );
    let checks = json(&checks);
    assert_eq!(checks["finding"], "confirmed", "{checks}");
    assert_eq!(checks["checks"][0]["runner_trusted"], true);
    // The CLI reads the same stored checks: the ladder, claim gate and labels are one path.
    let shown = cli(&dir, &hermit, &["confirm", &failure, "--reruns", "1"]);
    assert_eq!(
        shown.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&shown.stdout)
    );
    let log: Vec<String> = errors.try_iter().collect();
    assert!(
        log.iter()
            .any(|l| l.contains(&format!("confirmation of {failure} started by ci bot (t:"))),
        "{log:?}"
    );
    assert!(
        log.iter().all(|l| !l.contains(RUN) && !l.contains(READ)),
        "{log:?}"
    );
}

#[test]
fn it_is_off_unless_the_flag_is_given() {
    let (dir, hermit) = project("off");
    let (_server, base, _) = serve(&dir, &hermit, &[], &tokens());
    let failure = the_failure(&base);

    let me = json(&http("GET", &format!("{base}/api/me"), RUN).1);
    let (status, body) = http(
        "POST",
        &format!("{base}/api/failures/{failure}/confirm"),
        RUN,
    );

    assert_eq!(me["can"]["confirm"], false);
    assert_eq!(status, 403, "{body}");
    assert_eq!(
        json(&body)["error"],
        "confirmation from the browser is not enabled on this server"
    );
}

#[test]
fn a_cap_above_four_is_refused_at_start_up() {
    let dir = scratch("cap");
    let mut child = Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(["serve", "--addr", "127.0.0.1:0", "--max-confirmations", "5"])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary runs");

    // A server that did start would listen for ever, so wait a bounded time for it to refuse and fail if it does not.
    let status = wait_for_exit(&mut child, Duration::from_secs(20)).unwrap_or_else(|| {
        let _ = child.kill();
        let _ = child.wait();
        panic!("the server did not refuse a cap above four");
    });
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    assert_ne!(status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("confirmation cap"), "{stderr}");
}

/// The exit status once `child` has exited, or `None` if it is still running after `limit`.
fn wait_for_exit(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().expect("can wait") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A Hermit that takes two seconds to start while a file named `sleep` sits beside it, and leaves a file named
/// `started` when it does.
const SLOW_HERMIT: &str = "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 'fake-hermit 0.0'; exit 0; }\n[ -f \"$(dirname \"$0\")/sleep\" ] && { echo $$ > \"$(dirname \"$0\")/hermit.pid\"; touch \"$(dirname \"$0\")/started\"; sleep 2; }\nshift\nseed=0\nwhile [ \"$1\" != \"--\" ]; do case \"$1\" in --sched-seed=*) seed=${1#--sched-seed=};; esac; shift; done\nshift\nFAKE_SCHED_SEED=$seed exec \"$@\"\n";

#[test]
fn ctrl_c_stops_a_running_confirmation_promptly_and_stores_the_rest_as_skipped() {
    stops_a_running_confirmation_on("INT", "sigint");
}

#[test]
fn sigterm_stops_a_running_confirmation_promptly_and_stores_the_rest_as_skipped() {
    // What systemd sends by default when it stops the service.
    stops_a_running_confirmation_on("TERM", "sigterm");
}

fn stops_a_running_confirmation_on(signal: &str, name: &str) {
    // Arrange: a stored failure, then a Hermit that takes two seconds per start, so a whole confirmation (three
    // reruns and a replay) would take about eight.
    let (dir, hermit) = project(name);
    let (mut server, base, _errors) =
        serve(&dir, &hermit, &["--max-confirmations", "1"], &tokens());
    let failure = the_failure(&base);
    std::fs::write(dir.join("sleep"), "").unwrap();
    let (status, body) = http(
        "POST",
        &format!("{base}/api/failures/{failure}/confirm"),
        RUN,
    );
    assert_eq!(status, 202, "{body}");
    let started = dir.join("started");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !started.exists() {
        assert!(Instant::now() < deadline, "the confirmation never started");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Act: the signal, as a terminal (INT) or a service manager (TERM) sends it.
    let asked = Instant::now();
    let sent = Command::new("kill")
        .args([&format!("-{signal}"), &server.0.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(sent.success());
    let exit = wait_for_exit(&mut server.0, Duration::from_secs(30));

    // Assert: it ended well before the confirmation could have, and what ran is stored with the rest skipped.
    assert!(exit.is_some(), "the server did not exit");
    assert!(
        asked.elapsed() < Duration::from_secs(6),
        "took {:?}; a whole confirmation takes about eight seconds",
        asked.elapsed()
    );
    std::fs::remove_file(dir.join("sleep")).unwrap();
    let (_server2, base2, _) = serve(&dir, &hermit, &["--max-confirmations", "1"], &tokens());
    let (_, checks) = http(
        "GET",
        &format!("{base2}/api/failures/{failure}/checks"),
        READ,
    );
    let checks = json(&checks);
    let all = checks["checks"].as_array().unwrap();
    assert!(
        !all.is_empty(),
        "a partial confirmation is stored: {checks}"
    );
    assert!(all.iter().any(|c| c["status"] == "skipped"), "{checks}");
    // The run in progress finished (it cannot be taken back), and its check is stored inconclusive, not as a result.
    assert_eq!(all[0]["status"], "inconclusive", "{checks}");
}

#[test]
fn a_failure_of_a_simulation_target_is_refused_in_fixed_words() {
    // Arrange: a stored failure of a built-in simulation target.
    let dir = scratch("sim");
    let hermit = dir.join("fake-hermit");
    std::fs::write(&hermit, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&hermit, std::fs::Permissions::from_mode(0o755)).unwrap();
    let test = cli(&dir, &hermit, &["test", "lock-lease", "--seed", "7"]);
    assert_eq!(test.status.code(), Some(1));
    let (_server, base, _) = serve(&dir, &hermit, &["--max-confirmations", "1"], &tokens());
    let failure = the_failure(&base);

    // Act
    let (status, body) = http(
        "POST",
        &format!("{base}/api/failures/{failure}/confirm"),
        RUN,
    );

    // Assert
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        json(&body)["error"],
        "only a failure of a process target can be confirmed, and this one is not"
    );
    let (_, state) = http(
        "GET",
        &format!("{base}/api/failures/{failure}/confirm"),
        READ,
    );
    assert_eq!(json(&state)["state"], "idle");
}

#[test]
fn a_server_without_hermit_answers_with_stored_inconclusive_checks_and_does_not_hang() {
    let (dir, hermit) = project("nohermit");
    let missing = dir.join("no-such-hermit");
    let _ = hermit;
    let (_server, base, _) = serve(&dir, &missing, &["--max-confirmations", "1"], &tokens());
    let failure = the_failure(&base);

    let (status, body) = http(
        "POST",
        &format!("{base}/api/failures/{failure}/confirm"),
        RUN,
    );
    assert_eq!(status, 202, "{body}");
    wait_idle(&base, &failure);

    let (_, checks) = http(
        "GET",
        &format!("{base}/api/failures/{failure}/checks"),
        READ,
    );
    let checks = json(&checks);
    assert_eq!(checks["finding"], "candidate", "{checks}");
    let all = checks["checks"].as_array().unwrap();
    assert!(
        !all.is_empty() && all.iter().all(|c| c["status"] == "inconclusive"),
        "{checks}"
    );
}

#[test]
fn a_stop_that_also_kills_the_running_hermit_does_not_withdraw_a_confirmed_finding() {
    // Arrange: a failure confirmed once (so the finding is "confirmed"), then a slow Hermit for the second.
    let (dir, hermit) = project("group-kill");
    let (mut server, base, errors) = serve(&dir, &hermit, &["--max-confirmations", "1"], &tokens());
    let failure = the_failure(&base);
    let post = || {
        http(
            "POST",
            &format!("{base}/api/failures/{failure}/confirm"),
            RUN,
        )
    };
    assert_eq!(post().0, 202);
    wait_idle(&base, &failure);
    let before = json(
        &http(
            "GET",
            &format!("{base}/api/failures/{failure}/checks"),
            READ,
        )
        .1,
    );
    assert_eq!(before["finding"], "confirmed", "{before}");
    std::fs::write(dir.join("sleep"), "").unwrap();
    assert_eq!(post().0, 202);
    let started = dir.join("started");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !started.exists() {
        assert!(Instant::now() < deadline, "the confirmation never started");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Act: SIGTERM to the server and to the Hermit it started, in one call, as systemd's default control-group stop does
    // (the Hermit is not in the server's process group, so the test names it by the pid it wrote).
    let hermit_pid = std::fs::read_to_string(dir.join("hermit.pid")).expect("the pid file");
    let sent = Command::new("kill")
        .args(["-TERM", &server.0.id().to_string(), hermit_pid.trim()])
        .status()
        .expect("kill runs");
    assert!(sent.success());
    assert!(wait_for_exit(&mut server.0, Duration::from_secs(30)).is_some());
    // The log says the confirmation was stopped, not that it "finished: confirmed" on the strength of an earlier one.
    let mut log = Vec::new();
    while let Ok(line) = errors.recv_timeout(Duration::from_secs(3)) {
        log.push(line);
    }
    assert!(
        log.iter()
            .any(|l| l.contains(&format!("confirmation of {failure} by"))
                && l.contains("stopped early: cancelled")),
        "{log:?}"
    );

    // Assert: the run that was cut is stored inconclusive and the finding is where the first confirmation left it.
    std::fs::remove_file(dir.join("sleep")).unwrap();
    let (_second, base2, _) = serve(&dir, &hermit, &["--max-confirmations", "1"], &tokens());
    let after = json(
        &http(
            "GET",
            &format!("{base2}/api/failures/{failure}/checks"),
            READ,
        )
        .1,
    );
    assert_eq!(after["finding"], "confirmed", "{after}");
    let all = after["checks"].as_array().unwrap();
    let newest = all.last().unwrap()["confirmation_id"].clone();
    let of_newest: Vec<&Value> = all
        .iter()
        .filter(|c| c["confirmation_id"] == newest)
        .collect();
    assert_ne!(newest, after["confirmation_id"], "{after}");
    assert!(of_newest.iter().all(|c| c["status"] != "fail"), "{after}");
    assert_eq!(of_newest[0]["status"], "skipped", "{after}");
}
