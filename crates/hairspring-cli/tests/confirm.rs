//! `hairspring confirm`: the confirmation pipeline on a stored failure or a bundle, its stored results, and the summary
//! `show` gives of them.
#![cfg(unix)]
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FAKE_HERMIT: &str = "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 'fake-hermit 0.0'; exit 0; }\nshift\nseed=0\nwhile [ \"$1\" != \"--\" ]; do case \"$1\" in --sched-seed=*) seed=${1#--sched-seed=};; esac; shift; done\nshift\nFAKE_SCHED_SEED=$seed exec \"$@\"\n";
const BUG: &str = "[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: lost update'; exit 1; }; echo ok";
const PROJECT: &str = "[target.svc]\nkind = \"process\"\ncommand = [\"/bin/sh\", \"-c\", \"[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: lost update'; exit 1; }; echo ok\"]\n[target.svc.budget]\nmax_trials = 20\n[target.fixed]\nkind = \"process\"\ncommand = [\"/bin/sh\", \"-c\", \"echo fixed\"]\n";

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("hairspring-confirm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, hermit: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(args)
        .current_dir(dir)
        .env("HAIRSPRING_HERMIT", hermit)
        .output()
        .expect("binary runs")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

struct Project {
    dir: PathBuf,
    hermit: PathBuf,
    run_id: String,
    bundle_file: PathBuf,
}

/// A project whose seed 3 fails, one stored run of it, and its bundle file.
fn project(name: &str) -> Project {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch(name);
    let hermit = dir.join("fake-hermit");
    std::fs::write(&hermit, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&hermit, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("hairspring.toml"), PROJECT).unwrap();
    let o = run(&dir, &hermit, &["test", "svc", "--seed", "1"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let listing = run(&dir, &hermit, &["results", "--limit", "1"]);
    let run_id = text(&listing)
        .split_whitespace()
        .next()
        .expect("a run id")
        .to_string();
    let bundle_file = std::fs::read_dir(dir.join(".hairspring/bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    Project {
        dir,
        hermit,
        run_id,
        bundle_file,
    }
}

impl Project {
    fn run(&self, args: &[&str]) -> Output {
        run(&self.dir, &self.hermit, args)
    }
}

#[test]
fn confirming_a_run_stores_the_checks_and_says_confirmed() {
    let p = project("run");
    let o = p.run(&["confirm", &p.run_id]);
    let out = text(&o);
    assert_eq!(
        o.status.code(),
        Some(1),
        "a confirmed failure exits 1: {out}"
    );
    assert!(
        out.contains("same_seed_rerun") && out.contains("bundle_replay"),
        "{out}"
    );
    assert!(out.contains("3 passed, 0 failed, 0 inconclusive"), "{out}");
    assert!(out.contains("command: fake-hermit run --epoch="), "{out}");
    assert!(
        !out.contains(p.dir.to_str().unwrap()),
        "no host path in the stored command: {out}"
    );
    assert!(out.contains("--sched-seed=3"), "{out}");
    assert!(out.contains("finding: confirmed"), "{out}");
    assert!(!out.to_lowercase().contains("found"), "{out}");
}

#[test]
fn show_summarises_the_stored_checks() {
    let p = project("show");
    let before = text(&p.run(&["show", &p.run_id]));
    assert!(
        !before.contains("finding"),
        "nothing is said before a confirmation: {before}"
    );
    p.run(&["confirm", &p.run_id]);
    let after = text(&p.run(&["show", &p.run_id]));
    assert!(after.contains("finding fail_"), "{after}");
    assert!(after.contains("confirmed"), "{after}");
    assert!(after.contains("same_seed_rerun pass"), "{after}");

    let json = p.run(&["show", &p.run_id, "--json"]);
    let first = String::from_utf8_lossy(&json.stdout)
        .lines()
        .next()
        .unwrap()
        .to_string();
    let v: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(v["failures"][0]["finding"]["status"], "confirmed");
    assert_eq!(v["failures"][0]["finding"]["checks"], 2);
}

#[test]
fn a_second_confirmation_adds_checks() {
    let p = project("twice");
    p.run(&["confirm", &p.run_id]);
    p.run(&["confirm", &p.run_id, "--reruns", "5"]);
    let json = p.run(&["show", &p.run_id, "--json"]);
    let first = String::from_utf8_lossy(&json.stdout)
        .lines()
        .next()
        .unwrap()
        .to_string();
    let v: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(v["failures"][0]["finding"]["checks"], 4);
}

#[test]
fn a_control_that_does_not_fail_is_reported_for_what_it_is() {
    let p = project("control");
    let o = p.run(&["confirm", &p.run_id, "--control", "fixed"]);
    let out = text(&o);
    assert_eq!(o.status.code(), Some(1), "{out}");
    assert!(
        out.contains("fixed_control") && out.contains("pass"),
        "{out}"
    );
    assert!(out.contains("does not fail at this seed"), "{out}");
    assert!(!out.to_lowercase().contains("same interleaving"), "{out}");
}

#[test]
fn a_control_that_still_fails_is_flagged_and_exits_inconclusive() {
    let p = project("control-fails");
    let o = p.run(&["confirm", &p.run_id, "--control", "svc"]);
    let out = text(&o);
    assert_eq!(o.status.code(), Some(3), "{out}");
    assert!(out.contains("control failed"), "{out}");
    assert!(out.contains("finding: control_failed"), "{out}");
}

#[test]
fn a_bundle_file_is_confirmed_without_storing_anything() {
    let p = project("bundle");
    let file = p.bundle_file.to_str().unwrap();
    let o = p.run(&["confirm", file]);
    let out = text(&o);
    assert_eq!(o.status.code(), Some(1), "{out}");
    assert!(out.contains("not stored"), "{out}");
    assert!(out.contains("finding: confirmed"), "{out}");
    let show = text(&p.run(&["show", &p.run_id]));
    assert!(!show.contains("finding"), "{show}");
}

fn edited_bundle(p: &Project, name: &str, edit: impl Fn(String) -> String) -> String {
    let body = std::fs::read_to_string(&p.bundle_file).unwrap();
    let path = p.dir.join(name);
    std::fs::write(&path, edit(body)).unwrap();
    path.to_str().unwrap().to_string()
}

#[test]
fn a_miss_on_a_changed_environment_is_inconclusive_with_its_own_exit_code() {
    let p = project("env");
    let file = edited_bundle(&p, "env.hairspring", |b| {
        b.replace("param.sched_seed=3", "param.sched_seed=1")
            .replacen("\npath=", "\nparam.host.kernel_release=1.2.3-old\npath=", 1)
    });
    let o = p.run(&["confirm", &file]);
    let out = text(&o);
    assert_eq!(o.status.code(), Some(3), "{out}");
    assert!(out.contains("inconclusive: environment differs"), "{out}");
    assert!(out.contains("does not show the bug is fixed"), "{out}");
}

#[test]
fn a_miss_on_an_unchanged_host_is_not_reproducible_and_exits_zero() {
    let p = project("fixed");
    let file = edited_bundle(&p, "fixed.hairspring", |b| {
        b.replace("param.sched_seed=3", "param.sched_seed=1")
    });
    let o = p.run(&["confirm", &file]);
    let out = text(&o);
    assert_eq!(o.status.code(), Some(0), "{out}");
    assert!(out.contains("finding: not_reproducible"), "{out}");
}

#[test]
fn a_changed_loader_is_recorded_as_inconclusive_unless_allowed() {
    let p = project("loader");
    let sha = "a".repeat(64);
    let file = edited_bundle(&p, "loader.hairspring", |b| {
        b.replacen(
            "\npath=",
            &format!("\nparam.host.loader.ld_so_cache.sha256={sha}\npath="),
            1,
        )
    });
    let o = p.run(&["confirm", &file]);
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(text(&o).contains("environment differs"), "{}", text(&o));
    let allowed = p.run(&["confirm", &file, "--allow-environment-change"]);
    assert_eq!(allowed.status.code(), Some(1), "{}", text(&allowed));
}

#[test]
fn limits_and_bad_input_are_refused_with_exit_two() {
    let p = project("limits");
    for args in [
        vec!["confirm", p.run_id.as_str(), "--reruns", "0"],
        vec!["confirm", p.run_id.as_str(), "--reruns", "11"],
        vec!["confirm", p.run_id.as_str(), "--wall-secs", "0"],
        vec!["confirm", p.run_id.as_str(), "--wall-secs", "601"],
        vec!["confirm", p.run_id.as_str(), "--control", "nope"],
        vec!["confirm", "run_0000000000000000"],
        vec!["confirm", "fail_0000000000000000"],
        vec!["confirm", "not-a-file"],
    ] {
        let o = p.run(&args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", text(&o));
    }
}

#[test]
fn json_output_is_the_stored_answer() {
    let p = project("json");
    let o = p.run(&["confirm", &p.run_id, "--json"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let v: Value = serde_json::from_slice(&o.stdout).expect("one JSON document");
    assert_eq!(v["finding"], "confirmed");
    assert_eq!(v["checks"][0]["kind"], "same_seed_rerun");
}

#[test]
fn the_help_lists_the_exit_codes() {
    let p = project("help");
    let help = text(&p.run(&["confirm", "--help"]));
    assert!(help.contains("Exit codes"), "{help}");
    assert!(
        help.contains("--reruns") && help.contains("--control"),
        "{help}"
    );
    assert!(help.contains("--strict-verify"), "{help}");
    let _ = BUG;
}

#[test]
fn a_strict_verify_that_cannot_run_here_is_recorded_not_assumed() {
    let p = project("verify");
    let o = p.run(&["confirm", &p.run_id, "--strict-verify"]);
    let out = text(&o);
    assert!(
        out.contains("strict_verify") && out.contains("inconclusive"),
        "{out}"
    );
    assert!(!out.contains("finding: verified"), "{out}");
}
