//! End-to-end tests: run the real `anthill` binary in a scratch directory.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "anthill-cli-{name}-{}-{}",
        std::process::id(),
        anthill_store::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn anthill(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_anthill"))
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
    let o = anthill(&d, &["targets"]);
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
    let o = anthill(&d, &["test", "lock-lease", "--seed", "7"]);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    let s = out(&o);
    assert!(
        s.contains("FAILED") && s.contains("lock-lease") && s.contains("anthill-bundle v2"),
        "{s}"
    );
    let bundles: Vec<_> = std::fs::read_dir(d.join(".anthill/bundles"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(bundles.len(), 1, "one bundle file written");
    assert!(std::fs::read_to_string(&bundles[0])
        .unwrap()
        .starts_with("anthill-bundle v2"));
}

#[test]
fn test_passes_the_fixed_target_with_exit_0() {
    let d = scratch("pass");
    let o = anthill(
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
    let j = anthill(
        &d,
        &["test", "lock-lease", "--seed", "7", "--format", "json"],
    );
    let v: serde_json::Value = serde_json::from_str(&out(&j)).expect("json output parses");
    assert_eq!(v[0]["target"], "lock-lease");
    assert_eq!(v[0]["outcome"]["status"], "failed");
    let x = anthill(
        &d,
        &["test", "lock-lease", "--seed", "7", "--format", "junit"],
    );
    assert!(out(&x).starts_with("<?xml") && out(&x).contains("<failure"));
    let s = anthill(
        &d,
        &["test", "lock-lease", "--seed", "7", "--format", "sarif"],
    );
    let v: serde_json::Value = serde_json::from_str(&out(&s)).unwrap();
    assert_eq!(v["version"], "2.1.0");
}

#[test]
fn replay_reproduces_a_bundle_file_and_exits_1() {
    let d = scratch("replay");
    anthill(&d, &["test", "pubsub-buggy", "--seed", "5"]);
    let file = std::fs::read_dir(d.join(".anthill/bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let o = anthill(&d, &["replay", file.to_str().unwrap()]);
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
    std::fs::write(d.join("bad.anthill"), "not a bundle").unwrap();
    let o = anthill(&d, &["replay", "bad.anthill"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("bundle"), "{}", err(&o));
}

#[test]
fn results_and_show_read_back_stored_runs() {
    let d = scratch("results");
    anthill(&d, &["test", "lock-lease", "--seed", "7"]);
    let o = anthill(&d, &["results"]);
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
    let shown = anthill(&d, &["show", &id]);
    assert_eq!(shown.status.code(), Some(0));
    assert!(out(&shown).contains("FAILED") && out(&shown).contains("anthill-bundle"));
    assert_eq!(
        anthill(&d, &["show", "run_doesnotexist"]).status.code(),
        Some(2)
    );
}

#[test]
fn doctor_passes_for_demo_targets() {
    let d = scratch("doctor");
    let o = anthill(
        &d,
        &["doctor", "lock-lease", "--seeds", "3", "--steps", "40"],
    );
    assert_eq!(o.status.code(), Some(0), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("deterministic"));
}

#[test]
fn init_writes_a_config_once() {
    let d = scratch("init");
    assert_eq!(anthill(&d, &["init"]).status.code(), Some(0));
    let text = std::fs::read_to_string(d.join("anthill.toml")).unwrap();
    assert!(text.contains("[target."));
    let again = anthill(&d, &["init"]);
    assert_eq!(again.status.code(), Some(2), "refuses to overwrite");
    assert_eq!(
        std::fs::read_to_string(d.join("anthill.toml")).unwrap(),
        text
    );
    // the generated file is itself valid
    assert_eq!(anthill(&d, &["targets"]).status.code(), Some(0));
}

#[test]
fn unknown_target_and_bad_config_exit_2() {
    let d = scratch("errors");
    let o = anthill(&d, &["test", "no-such-target"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("no-such-target"));
    std::fs::write(d.join("anthill.toml"), "[target.x]\nkind = \"sim\"\n").unwrap();
    let o = anthill(&d, &["targets"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("sim"), "{}", err(&o));
}

#[test]
fn config_file_targets_are_runnable_aliases() {
    let d = scratch("alias");
    std::fs::write(d.join("anthill.toml"), "[target.my-lock]\nkind = \"sim\"\nsim = \"lock-lease\"\n[target.my-lock.budget]\nmax_steps = 100000\n").unwrap();
    let o = anthill(&d, &["test", "my-lock", "--seed", "7"]);
    assert_eq!(o.status.code(), Some(1), "{}{}", out(&o), err(&o));
    assert!(out(&o).contains("my-lock"));
}
