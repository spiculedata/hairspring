//! End-to-end tests: run the real `hairspring` binary in a scratch directory.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-cli-{name}-{}-{}",
        std::process::id(),
        hairspring_store::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hairspring(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("binary runs")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn targets_lists_builtin_demos() {
    let d = scratch("targets");
    let o = hairspring(&d, &["targets"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let s = out(&o);
    assert!(
        s.contains("lock-lease") && s.contains("pubsub-buggy") && s.contains("pubsub-atomic"),
        "{s}"
    );
}

#[test]
fn test_finds_the_bug_exits_1_and_writes_a_bundle_file() {
    let d = scratch("fail");
    let o = hairspring(&d, &["test", "lock-lease", "--seed", "7"]);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    let s = out(&o);
    assert!(
        s.contains("FAILED") && s.contains("lock-lease") && s.contains("hairspring-bundle v2"),
        "{s}"
    );
    let bundles: Vec<_> = std::fs::read_dir(d.join(".hairspring/bundles"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(bundles.len(), 1, "one bundle file written");
    assert!(std::fs::read_to_string(&bundles[0])
        .unwrap()
        .starts_with("hairspring-bundle v2"));
}

#[test]
fn test_passes_the_fixed_target_with_exit_0() {
    let d = scratch("pass");
    let o = hairspring(
        &d,
        &[
            "test",
            "pubsub-atomic",
            "--seed",
            "1",
            "--budget-steps",
            "20000",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("PASSED"));
}

#[test]
fn machine_readable_formats_are_valid() {
    let d = scratch("formats");
    let j = hairspring(
        &d,
        &["test", "lock-lease", "--seed", "7", "--format", "json"],
    );
    let v: serde_json::Value = serde_json::from_str(&out(&j)).expect("json output parses");
    assert_eq!(v[0]["target"], "lock-lease");
    assert_eq!(v[0]["outcome"]["status"], "failed");
    let x = hairspring(
        &d,
        &["test", "lock-lease", "--seed", "7", "--format", "junit"],
    );
    assert!(out(&x).starts_with("<?xml") && out(&x).contains("<failure"));
    let s = hairspring(
        &d,
        &["test", "lock-lease", "--seed", "7", "--format", "sarif"],
    );
    let v: serde_json::Value = serde_json::from_str(&out(&s)).unwrap();
    assert_eq!(v["version"], "2.1.0");
}

#[test]
fn replay_reproduces_a_bundle_file_and_exits_1() {
    let d = scratch("replay");
    hairspring(&d, &["test", "pubsub-buggy", "--seed", "5"]);
    let file = std::fs::read_dir(d.join(".hairspring/bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let o = hairspring(&d, &["replay", file.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    assert!(
        out(&o).contains("reproduced") && out(&o).contains("Deliver"),
        "{}",
        out(&o)
    );
}

#[test]
fn replay_rejects_garbage_with_exit_2() {
    let d = scratch("garbage");
    std::fs::write(d.join("bad.hairspring"), "not a bundle").unwrap();
    let o = hairspring(&d, &["replay", "bad.hairspring"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("bundle"), "{}", err(&o));
}

#[test]
fn results_and_show_read_back_stored_runs() {
    let d = scratch("results");
    hairspring(&d, &["test", "lock-lease", "--seed", "7"]);
    let o = hairspring(&d, &["results"]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let line = out(&o);
    assert!(
        line.contains("lock-lease") && line.contains("failed"),
        "{line}"
    );
    let id = line
        .split_whitespace()
        .find(|w| w.starts_with("run_"))
        .expect("run id printed")
        .to_string();
    let shown = hairspring(&d, &["show", &id]);
    assert_eq!(shown.status.code(), Some(0));
    assert!(out(&shown).contains("FAILED") && out(&shown).contains("hairspring-bundle"));
    assert_eq!(
        hairspring(&d, &["show", "run_doesnotexist"]).status.code(),
        Some(2)
    );
}

#[test]
fn doctor_passes_for_demo_targets() {
    let d = scratch("doctor");
    let o = hairspring(
        &d,
        &["doctor", "lock-lease", "--seeds", "3", "--steps", "40"],
    );
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("deterministic"));
}

#[test]
fn init_writes_a_config_once() {
    let d = scratch("init");
    assert_eq!(hairspring(&d, &["init"]).status.code(), Some(0));
    let text = std::fs::read_to_string(d.join("hairspring.toml")).unwrap();
    assert!(text.contains("[target."));
    let again = hairspring(&d, &["init"]);
    assert_eq!(again.status.code(), Some(2), "refuses to overwrite");
    assert_eq!(
        std::fs::read_to_string(d.join("hairspring.toml")).unwrap(),
        text
    );
    // the generated file is itself valid
    assert_eq!(hairspring(&d, &["targets"]).status.code(), Some(0));
}

#[test]
fn unknown_target_and_bad_config_exit_2() {
    let d = scratch("errors");
    let o = hairspring(&d, &["test", "no-such-target"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("no-such-target"));
    std::fs::write(d.join("hairspring.toml"), "[target.x]\nkind = \"sim\"\n").unwrap();
    let o = hairspring(&d, &["targets"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("sim"), "{}", err(&o));
}

#[test]
fn config_file_targets_are_runnable_aliases() {
    let d = scratch("alias");
    std::fs::write(d.join("hairspring.toml"), "[target.my-lock]\nkind = \"sim\"\nsim = \"lock-lease\"\n[target.my-lock.budget]\nmax_steps = 100000\n").unwrap();
    let o = hairspring(&d, &["test", "my-lock", "--seed", "7"]);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("my-lock"));
}

// ---- serve / worker / mcp / token ----
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

struct Guard(Child);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Guard {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hairspring"));
    cmd.args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    Guard(cmd.spawn().expect("spawn"))
}

/// Reads stdout lines until one contains `needle`, with a timeout.
fn wait_for_line(g: &mut Guard, needle: &str) -> String {
    let stdout = g.0.stdout.take().expect("stdout piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) if line.contains(needle) => return line,
            Ok(_) => continue,
            Err(_) => panic!("never saw `{needle}` in output"),
        }
    }
}

fn http(method: &str, url: &str, token: Option<&str>, body: Option<&str>) -> (u16, String) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let mut resp = match method {
        "GET" => {
            let mut r = agent.get(url);
            if let Some(t) = token {
                r = r.header("Authorization", format!("Bearer {t}"));
            }
            r.call()
        }
        _ => {
            let mut r = agent.post(url).header("Content-Type", "application/json");
            if let Some(t) = token {
                r = r.header("Authorization", format!("Bearer {t}"));
            }
            r.send(body.unwrap_or("{}"))
        }
    }
    .expect("http ok");
    (
        resp.status().as_u16(),
        resp.body_mut().read_to_string().unwrap_or_default(),
    )
}

fn url_of(line: &str) -> String {
    line.split_whitespace()
        .find(|w| w.starts_with("http://"))
        .expect("url in line")
        .to_string()
}

#[test]
fn serve_runs_a_run_end_to_end_over_http() {
    let d = scratch("serve");
    let mut g = spawn(&d, &["serve", "--addr", "127.0.0.1:0"], &[]);
    let base = url_of(&wait_for_line(&mut g, "listening on"));
    assert_eq!(
        http("GET", &format!("{base}/api/health"), None, None).0,
        200
    );
    let (status, body) = http(
        "POST",
        &format!("{base}/api/runs"),
        None,
        Some("{\"target\":\"lock-lease\",\"seed\":7}"),
    );
    assert_eq!(status, 201, "{body}");
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let (_, b) = http("GET", &format!("{base}/api/runs/{id}"), None, None);
        if serde_json::from_str::<serde_json::Value>(&b).unwrap()["run"]["state"] == "failed" {
            break;
        }
        assert!(Instant::now() < deadline, "run never finished: {b}");
        std::thread::sleep(Duration::from_millis(100));
    }
    // results persisted by the server are visible to the CLI afterwards
    assert!(out(&hairspring(&d, &["results"])).contains("lock-lease"));
}

#[test]
fn serve_refuses_a_public_address_without_tokens() {
    let d = scratch("serve-public");
    let o = hairspring(&d, &["serve", "--addr", "0.0.0.0:0"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("token"), "{}", err(&o));
}

#[test]
fn serve_enforces_tokens_from_the_environment() {
    let d = scratch("serve-auth");
    let tok = "env-token-0123456789abcdef";
    let mut g = spawn(
        &d,
        &["serve", "--addr", "127.0.0.1:0"],
        &[("HAIRSPRING_TOKENS", &format!("{tok}:admin"))],
    );
    let base = url_of(&wait_for_line(&mut g, "listening on"));
    assert_eq!(
        http("GET", &format!("{base}/api/targets"), None, None).0,
        401
    );
    assert_eq!(
        http("GET", &format!("{base}/api/targets"), Some(tok), None).0,
        200
    );
}

#[test]
fn token_command_prints_distinct_strong_tokens() {
    let d = scratch("token");
    let a = out(&hairspring(&d, &["token", "--role", "run"]));
    let b = out(&hairspring(&d, &["token", "--role", "run"]));
    let tok = |s: &str| {
        s.split_whitespace()
            .find(|w| w.contains(":run"))
            .map(|w| w.trim_start_matches("HAIRSPRING_TOKENS=").to_string())
            .unwrap_or_default()
    };
    let (ta, tb) = (tok(&a), tok(&b));
    assert!(ta.len() >= 40 && ta.ends_with(":run"), "{a}");
    assert_ne!(ta, tb);
    assert_eq!(
        hairspring(&d, &["token", "--role", "root"]).status.code(),
        Some(2)
    );
}

#[test]
fn mcp_stdio_serves_the_agent_loop() {
    let d = scratch("mcp");
    let mut g = spawn(&d, &["mcp"], &[]);
    let mut stdin = g.0.stdin.take().unwrap();
    let msgs = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"start_run","arguments":{"target":"lock-lease","seed":7}}}"#,
    ];
    for m in msgs {
        writeln!(stdin, "{m}").unwrap();
    }
    drop(stdin);
    let reader = BufReader::new(g.0.stdout.take().unwrap());
    let lines: Vec<serde_json::Value> = reader
        .lines()
        .map_while(Result::ok)
        .map(|l| serde_json::from_str(&l).unwrap())
        .collect();
    assert_eq!(
        lines.len(),
        2,
        "one reply per request, none for the notification"
    );
    assert_eq!(lines[0]["result"]["serverInfo"]["name"], "hairspring");
    assert_eq!(
        lines[1]["result"]["structuredContent"]["run"]["state"],
        "failed"
    );
}

#[test]
fn a_cli_worker_executes_runs_for_a_server_without_embedded_workers() {
    let d = scratch("worker");
    let tok = "worker-token-0123456789abcdef";
    let mut srv = spawn(
        &d,
        &["serve", "--addr", "127.0.0.1:0", "--workers", "0"],
        &[("HAIRSPRING_TOKENS", &format!("{tok}:admin"))],
    );
    let base = url_of(&wait_for_line(&mut srv, "listening on"));
    let _worker = spawn(
        &d,
        &["worker", "--server", &base, "--poll-ms", "50"],
        &[("HAIRSPRING_TOKEN", tok)],
    );
    let (status, body) = http(
        "POST",
        &format!("{base}/api/runs"),
        Some(tok),
        Some("{\"target\":\"pubsub-buggy\",\"seed\":5}"),
    );
    assert_eq!(status, 201, "{body}");
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (_, b) = http("GET", &format!("{base}/api/runs/{id}"), Some(tok), None);
        if serde_json::from_str::<serde_json::Value>(&b).unwrap()["run"]["state"] == "failed" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "worker never completed the run: {b}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ---- process targets via the Hermit backend (fake hermit) ----
fn write_exe(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn process_project(name: &str) -> (PathBuf, String) {
    let d = scratch(name);
    let fake = d.join("fake-hermit");
    write_exe(&fake, "#!/bin/sh\nshift\nseed=0\nwhile [ \"$1\" != \"--\" ]; do case \"$1\" in --sched-seed=*) seed=${1#--sched-seed=};; esac; shift; done\nshift\nFAKE_SCHED_SEED=$seed exec \"$@\"\n");
    std::fs::write(
        d.join("hairspring.toml"),
        "[target.svc]\nkind = \"process\"\ncommand = [\"/bin/sh\", \"-c\", \"[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: lost update'; exit 1; }; echo ok\"]\n[target.svc.budget]\nmax_trials = 20\n",
    )
    .unwrap();
    let path = fake.to_str().unwrap().to_string();
    (d, path)
}

fn hairspring_env(dir: &Path, args: &[&str], hermit: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(args)
        .current_dir(dir)
        .env("HAIRSPRING_HERMIT", hermit)
        .output()
        .expect("binary runs")
}

#[test]
fn process_targets_are_explored_replayed_and_checked_through_the_cli() {
    let (d, hermit) = process_project("proc");
    let o = hairspring_env(&d, &["test", "svc", "--seed", "1"], &hermit);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    let s = out(&o);
    assert!(
        s.contains("FAILED")
            && s.contains("sched-seed 3")
            && s.contains("lost update")
            && s.contains("backend=hermit"),
        "{s}"
    );
    let file = std::fs::read_dir(d.join(".hairspring/bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let r = hairspring_env(&d, &["replay", file.to_str().unwrap()], &hermit);
    assert_eq!(r.status.code(), Some(1), "{}{}", out(&r), err(&r));
    assert!(
        out(&r).contains("output matches the recording: yes"),
        "{}",
        out(&r)
    );
    let doc = hairspring_env(&d, &["doctor", "svc", "--seeds", "3"], &hermit);
    assert_eq!(doc.status.code(), Some(0), "{}{}", out(&doc), err(&doc));
}

#[test]
fn process_targets_report_a_missing_hermit_clearly() {
    let (d, _) = process_project("noherm");
    let o = hairspring_env(&d, &["test", "svc"], "/definitely/not/hermit");
    assert_eq!(o.status.code(), Some(2), "{}{}", out(&o), err(&o));
    assert!(
        out(&o).contains("hermit executable not found")
            || err(&o).contains("hermit executable not found"),
        "{}{}",
        out(&o),
        err(&o)
    );
}

// ---- wasm targets ----
const LOST_UPDATE_WAT: &str = r#"
(module
  (import "hairspring" "choose" (func $choose (param i32) (result i32)))
  (import "hairspring" "fail" (func $fail (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "lost update")
  (global $bal (mut i32) (i32.const 100))
  (global $atmp (mut i32) (i32.const 0))
  (global $btmp (mut i32) (i32.const 0))
  (global $apc (mut i32) (i32.const 0))
  (global $bpc (mut i32) (i32.const 0))
  (func $step_a
    (if (i32.eqz (global.get $apc)) (then (global.set $atmp (global.get $bal)) (global.set $apc (i32.const 1)) (return)))
    (if (i32.eq (global.get $apc) (i32.const 1)) (then (global.set $bal (i32.add (global.get $atmp) (i32.const 10))) (global.set $apc (i32.const 2)))))
  (func $step_b
    (if (i32.eqz (global.get $bpc)) (then (global.set $btmp (global.get $bal)) (global.set $bpc (i32.const 1)) (return)))
    (if (i32.eq (global.get $bpc) (i32.const 1)) (then (global.set $bal (i32.add (global.get $btmp) (i32.const 10))) (global.set $bpc (i32.const 2)))))
  (func (export "run") (result i32)
    (block $done (loop $l
      (br_if $done (i32.and (i32.eq (global.get $apc) (i32.const 2)) (i32.eq (global.get $bpc) (i32.const 2))))
      (if (i32.eq (call $choose (i32.const 2)) (i32.const 0)) (then (call $step_a)) (else (call $step_b)))
      (br $l)))
    (if (i32.ne (global.get $bal) (i32.const 120)) (then (call $fail (i32.const 0) (i32.const 11))))
    (i32.const 0)))
"#;

#[test]
fn wasm_targets_are_explored_shrunk_and_replayed_through_the_cli() {
    let d = scratch("wasm");
    std::fs::write(d.join("lost.wat"), LOST_UPDATE_WAT).unwrap();
    std::fs::write(d.join("hairspring.toml"), "[target.bank]\nkind = \"wasm\"\nmodule = \"lost.wat\"\ndescription = \"two writers, lost update\"\n[target.bank.budget]\nmax_trials = 200\n").unwrap();
    let o = hairspring(&d, &["test", "bank", "--seed", "1"]);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    let s = out(&o);
    assert!(
        s.contains("lost update") && s.contains("choose(2)") && s.contains("backend=wasm"),
        "{s}"
    );
    let file = std::fs::read_dir(d.join(".hairspring/bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let r = hairspring(&d, &["replay", file.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(1), "{}{}", out(&r), err(&r));
    assert!(out(&r).contains("reproduced"));
    assert_eq!(
        hairspring(&d, &["doctor", "bank", "--seeds", "5"])
            .status
            .code(),
        Some(0)
    );
}

// ---- PostgreSQL (runs only with HAIRSPRING_TEST_PG_URL) ----
#[test]
fn the_database_can_be_postgres_selected_by_environment() {
    let Ok(url) = std::env::var("HAIRSPRING_TEST_PG_URL") else {
        eprintln!("skipped: set HAIRSPRING_TEST_PG_URL");
        return;
    };
    let d = scratch("pg");
    let schema = format!("cli_{}_{}", std::process::id(), hairspring_store::now_ms());
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_hairspring"))
            .args(args)
            .current_dir(&d)
            .env("HAIRSPRING_DATABASE_URL", &url)
            .env("HAIRSPRING_DATABASE_SCHEMA", &schema)
            .output()
            .expect("binary runs")
    };
    let o = run(&["test", "lock-lease", "--seed", "7"]);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    assert!(
        !d.join(".hairspring/hairspring.db").exists(),
        "the local SQLite file must not be created when Postgres is configured"
    );
    let r = run(&["results"]);
    assert!(
        out(&r).contains("lock-lease") && out(&r).contains("failed"),
        "{}{}",
        out(&r),
        err(&r)
    );
    assert!(
        !err(&r).contains(&url) && !out(&r).contains(&url),
        "the connection URL (with its password) is never printed"
    );
}

#[test]
fn a_bad_database_url_fails_cleanly_without_echoing_secrets() {
    let d = scratch("pgbad");
    let o = Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(["results"])
        .current_dir(&d)
        .env(
            "HAIRSPRING_DATABASE_URL",
            "postgres://user:topsecret-pw@127.0.0.1:1/nope",
        )
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(!err(&o).contains("topsecret-pw"), "{}", err(&o));
}
