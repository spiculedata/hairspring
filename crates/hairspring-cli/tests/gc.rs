//! `hairspring gc`: reports first, removes only with the report's `--apply --cutoff T --expect-runs N --expect-digest D`, and
//! leaves everything it should alone.
use hairspring_store::{now_ms, NewFailure, NewRun, RunState, SqliteStore, Store};
use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

const LONG_AGO: i64 = 1_000;

struct Project {
    dir: PathBuf,
    db: PathBuf,
    bundles: PathBuf,
}

impl Project {
    fn new(name: &str) -> Project {
        let dir = std::env::temp_dir().join(format!(
            "hairspring-cli-gc-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let bundles = dir.join("bundles");
        std::fs::create_dir_all(&bundles).unwrap();
        Project {
            db: dir.join("results.db"),
            dir,
            bundles,
        }
    }

    fn store(&self) -> SqliteStore {
        SqliteStore::open(&self.db).unwrap()
    }

    /// A run that finished long ago in `state`; with `bundle`, one failure whose bundle file is also on disk.
    fn old_run(&self, state: RunState, bundle: Option<&str>) -> String {
        let s = self.store();
        let run = s
            .create_run(
                &NewRun {
                    target: "svc".into(),
                    backend: "sim".into(),
                    seed: 1,
                    config_json: "{}".into(),
                },
                LONG_AGO,
            )
            .unwrap();
        assert!(s
            .finish_run(&run.id, state, 5, 1, None, LONG_AGO + 1)
            .unwrap());
        if let Some(text) = bundle {
            let hash = s.put_blob(text.as_bytes()).unwrap();
            s.add_failure(
                &NewFailure {
                    run_id: run.id.clone(),
                    target: "svc".into(),
                    message: "m".into(),
                    original_len: 2,
                    shrunk_len: 1,
                    bundle_hash: hash.clone(),
                    steps_json: "[]".into(),
                },
                LONG_AGO + 1,
            )
            .unwrap();
            self.write_bundle_file(text);
        }
        run.id
    }

    fn write_bundle_file(&self, text: &str) -> PathBuf {
        let hash = hairspring_store::sha256_hex(text.as_bytes());
        let path = self.bundles.join(format!("svc-{}.hairspring", &hash[..8]));
        std::fs::write(&path, text).unwrap();
        path
    }

    fn run_exists(&self, id: &str) -> bool {
        self.store().get_run(id).unwrap().is_some()
    }

    fn gc<S: AsRef<std::ffi::OsStr>>(&self, args: &[S]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_hairspring"))
            .arg("--db")
            .arg(&self.db)
            .arg("gc")
            .arg("--bundle-dir")
            .arg(&self.bundles)
            .args(args)
            .current_dir(&self.dir)
            .output()
            .expect("binary runs")
    }

    /// `args` followed by the arguments that a dry run with `args` says remove exactly what it reports.
    fn apply_args(&self, args: &[&str]) -> Vec<String> {
        let mut dry = args.to_vec();
        dry.push("--json");
        let o = self.gc(&dry);
        assert_eq!(o.status.code(), Some(0), "{}", text(&o).1);
        let mut all: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        all.extend(
            json(&o)["apply_args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_owned()),
        );
        all
    }
}

/// Replaces the value after `flag` in `args`.
fn with(mut args: Vec<String>, flag: &str, value: &str) -> Vec<String> {
    let at = args.iter().position(|a| a == flag).unwrap() + 1;
    args[at] = value.to_owned();
    args
}

impl Drop for Project {
    fn drop(&mut self) {
        // A test may have made the directory read-only.
        #[cfg(unix)]
        let _ = std::fs::set_permissions(&self.bundles, perms(0o755));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(unix)]
fn perms(mode: u32) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(mode)
}

fn text(o: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn json(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {}", text(o).0))
}

#[test]
fn by_default_it_reports_and_removes_nothing() {
    let p = Project::new("dry");
    let a = p.old_run(RunState::Passed, Some("bundle a"));
    let b = p.old_run(RunState::Error, None);
    let o = p.gc(&["--older-than", "1h"]);
    let (out, err) = text(&o);
    assert_eq!(o.status.code(), Some(0), "{err}");
    assert!(
        out.contains("would remove (dry run, nothing has been changed)"),
        "{out}"
    );
    assert!(
        out.contains("runs                 2  (error 1, passed 1)"),
        "{out}"
    );
    assert!(out.contains("bundle files"), "{out}");
    assert!(
        out.contains("To remove exactly these 2 runs, run again with --apply --cutoff "),
        "{out}"
    );
    assert!(out.contains(" --expect-runs 2 --expect-digest "), "{out}");
    assert!(out.contains("by this computer's clock; cutoff "), "{out}");
    assert!(out.contains("the database file does not shrink"), "{out}");
    assert!(p.run_exists(&a) && p.run_exists(&b));
    assert_eq!(std::fs::read_dir(&p.bundles).unwrap().count(), 1);
}

#[test]
fn the_json_report_has_the_counts_and_says_it_is_a_dry_run() {
    let p = Project::new("json");
    p.old_run(RunState::Passed, Some("bundle j"));
    p.old_run(RunState::Failed, None);
    let o = p.gc(&["--older-than", "1h", "--json"]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o).1);
    let v = json(&o);
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["removed"], Value::Null);
    assert_eq!(v["would_remove"]["runs"], 1);
    assert_eq!(v["would_remove"]["failures"], 1);
    assert_eq!(v["would_remove"]["bundle_blobs"], 1);
    assert_eq!(v["would_remove"]["bundle_files"], 1);
    assert_eq!(v["would_remove"]["bundle_file_bytes"], 8);
    assert_eq!(v["would_remove"]["runs_by_state"]["passed"], 1);
    assert_eq!(v["kept"]["not_selected_failed"], 1);
    assert_eq!(
        v["states"],
        serde_json::json!(["cancelled", "error", "passed"])
    );
    assert!(v["not_deleted"].as_array().unwrap().len() >= 3);
}

#[test]
fn apply_without_the_expected_count_is_refused_after_printing_the_report() {
    let p = Project::new("noexpect");
    let a = p.old_run(RunState::Passed, None);
    let o = p.gc(&["--older-than", "1h", "--apply"]);
    let (out, err) = text(&o);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        out.contains("would remove (--apply was refused"),
        "the report comes first: {out}"
    );
    assert!(err.contains("without --cutoff "), "{err}");
    assert!(err.contains(" --expect-runs 1 --expect-digest "), "{err}");
    assert!(p.run_exists(&a));
    // The count alone is not enough either.
    let o = p.gc(&["--older-than", "1h", "--apply", "--expect-runs", "1"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o).1.contains("without --cutoff "), "{}", text(&o).1);
    assert!(p.run_exists(&a));
    // Nor the count and the digest without the cutoff they were found with.
    let mut args = p.apply_args(&["--older-than", "1h"]);
    let at = args.iter().position(|a| a == "--cutoff").unwrap();
    args.drain(at..at + 2);
    let o = p.gc(&args);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o).1.contains("without --cutoff "), "{}", text(&o).1);
    assert!(p.run_exists(&a));
}

#[test]
fn apply_with_a_different_count_is_refused() {
    let p = Project::new("wrongcount");
    let a = p.old_run(RunState::Passed, None);
    let b = p.old_run(RunState::Passed, None);
    let args = p.apply_args(&["--older-than", "1h"]);
    let o = p.gc(&with(args.clone(), "--expect-runs", "1"));
    assert_eq!(o.status.code(), Some(2));
    assert!(
        text(&o)
            .1
            .contains("--expect-runs is 1 but the report found 2 runs"),
        "{}",
        text(&o).1
    );
    assert!(p.run_exists(&a) && p.run_exists(&b));
    let mut args = with(args, "--expect-runs", "3");
    args.push("--json".into());
    let o = p.gc(&args);
    assert_eq!(o.status.code(), Some(2));
    assert!(json(&o)["refused"]
        .as_str()
        .unwrap()
        .contains("nothing was removed"));
}

/// The count matches but the runs do not: one run of the report is gone and another old run has appeared.
#[test]
fn apply_is_refused_when_the_runs_differ_although_the_count_matches() {
    let p = Project::new("otherruns");
    let a = p.old_run(RunState::Passed, None);
    let args = p.apply_args(&["--older-than", "1h"]);
    p.store()
        .retention_delete(std::slice::from_ref(&a), i64::MAX, false)
        .unwrap();
    let z = p.old_run(RunState::Passed, None);
    let o = p.gc(&args);
    let (_, err) = text(&o);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(
        err.contains("these are not the runs the report showed"),
        "{err}"
    );
    assert!(p.run_exists(&z), "the run the report did not show stays");
}

#[test]
fn a_cutoff_later_than_the_age_allows_is_refused() {
    let p = Project::new("latecutoff");
    let a = p.old_run(RunState::Passed, None);
    let soon = (now_ms() - 60_000).to_string();
    let o = p.gc(&["--older-than", "1h", "--cutoff", &soon]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        text(&o).1.contains("later than --older-than 1h allows"),
        "{}",
        text(&o).1
    );
    assert!(p.run_exists(&a));
}

/// The apply uses the report's cutoff, not one taken again later, so a run that has become old since the report is
/// not looked at, and the report's runs are removed.
#[test]
fn apply_uses_the_reported_cutoff_not_a_new_one() {
    let p = Project::new("pinned");
    let a = p.old_run(RunState::Passed, None);
    let args = p.apply_args(&["--older-than", "1h"]);
    let at = args.iter().position(|a| a == "--cutoff").unwrap() + 1;
    let cutoff: i64 = args[at].parse().unwrap();
    // A run that finished just after the cutoff: older than an hour by the time anyone applies a fresh scan later.
    let s = p.store();
    let late = s
        .create_run(
            &NewRun {
                target: "svc".into(),
                backend: "sim".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            cutoff,
        )
        .unwrap();
    s.finish_run(&late.id, RunState::Passed, 1, 1, None, cutoff + 1)
        .unwrap();
    drop(s);
    let o = p.gc(&args);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o).1);
    assert!(!p.run_exists(&a));
    assert!(p.run_exists(&late.id), "not old at the report's cutoff");
}

#[test]
fn expect_runs_without_apply_is_a_usage_error() {
    let p = Project::new("usage");
    let o = p.gc(&["--older-than", "1h", "--expect-runs", "1"]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_bad_age_is_an_error_and_a_missing_one_is_a_usage_error() {
    let p = Project::new("age");
    let o = p.gc(&["--older-than", "30"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o).1.contains("not an age"), "{}", text(&o).1);
    let o = p.gc::<&str>(&[]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn apply_removes_exactly_what_the_report_showed() {
    let p = Project::new("apply");
    let passed = p.old_run(RunState::Passed, Some("bundle p"));
    let cancelled = p.old_run(RunState::Cancelled, None);
    let failed = p.old_run(RunState::Failed, Some("bundle f"));
    let queued = {
        let s = p.store();
        s.create_run(
            &NewRun {
                target: "svc".into(),
                backend: "sim".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            LONG_AGO,
        )
        .unwrap()
        .id
    };
    let fresh = {
        let s = p.store();
        let r = s
            .create_run(
                &NewRun {
                    target: "svc".into(),
                    backend: "sim".into(),
                    seed: 1,
                    config_json: "{}".into(),
                },
                now_ms(),
            )
            .unwrap();
        s.finish_run(&r.id, RunState::Passed, 1, 1, None, now_ms())
            .unwrap();
        r.id
    };
    let unrelated = p.bundles.join("notes.txt");
    std::fs::write(&unrelated, "keep").unwrap();
    let passed_file = p.bundles.join(format!(
        "svc-{}.hairspring",
        &hairspring_store::sha256_hex(b"bundle p")[..8]
    ));
    let failed_file = p.bundles.join(format!(
        "svc-{}.hairspring",
        &hairspring_store::sha256_hex(b"bundle f")[..8]
    ));

    let mut args = p.apply_args(&["--older-than", "1h"]);
    args.push("--json".into());
    let o = p.gc(&args);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o).1);
    let v = json(&o);
    assert_eq!(v["dry_run"], false);
    assert_eq!(v["would_remove"]["runs"], 2);
    assert_eq!(v["kept_when_removing"], serde_json::json!({}));
    assert_eq!(v["removed"]["runs"], 2);
    assert_eq!(v["removed"]["bundle_files"], 1);
    assert_eq!(v["removed"]["bundle_blobs"], 1);
    assert_eq!(v["problems"], serde_json::json!([]));

    assert!(!p.run_exists(&passed) && !p.run_exists(&cancelled));
    assert!(
        p.run_exists(&failed),
        "failed runs are kept unless asked for"
    );
    assert!(p.run_exists(&queued), "a queued run is never removed");
    assert!(p.run_exists(&fresh), "a recent run is kept");
    assert!(!passed_file.exists());
    assert!(failed_file.exists() && unrelated.exists());
    let s = p.store();
    assert!(s.list_failures(&passed).unwrap().is_empty());

    let o = p.gc(&p.apply_args(&["--older-than", "1h", "--state", "failed"]));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o).1);
    assert!(!p.run_exists(&failed) && !failed_file.exists());
    assert!(p.run_exists(&queued) && p.run_exists(&fresh) && unrelated.exists());
}

/// While another gc holds the database's retention lock, a second one (a report or an apply) does nothing and says
/// why; once the lock is released it runs.
#[test]
fn a_second_gc_while_one_is_running_is_refused_and_changes_nothing() {
    let p = Project::new("busy");
    let passed = p.old_run(RunState::Passed, Some("bundle b"));
    let args = p.apply_args(&["--older-than", "1h"]);
    let holder = p.store();
    let lock = holder
        .retention_lock()
        .unwrap()
        .expect("nothing else holds it");

    for attempt in [
        vec!["--older-than".to_owned(), "1h".to_owned()],
        args.clone(),
    ] {
        let o = p.gc(&attempt);
        let (out, err) = text(&o);
        assert_eq!(o.status.code(), Some(2), "{out}{err}");
        assert!(
            err.contains("another hairspring gc is running against this database"),
            "{err}"
        );
        assert!(out.is_empty(), "no report: {out}");
        assert!(p.run_exists(&passed));
    }

    drop(lock);
    let o = p.gc(&args);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o).1);
    assert!(!p.run_exists(&passed));
}

#[test]
fn a_running_run_is_never_removed_whatever_states_are_asked_for() {
    let p = Project::new("running");
    let s = p.store();
    let run = s
        .create_run(
            &NewRun {
                target: "svc".into(),
                backend: "sim".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            LONG_AGO,
        )
        .unwrap();
    s.mark_running(&run.id, LONG_AGO + 1).unwrap();
    drop(s);
    let o = p.gc(&[
        "--older-than",
        "1h",
        "--state",
        "passed,failed,error,cancelled",
        "--json",
    ]);
    assert_eq!(o.status.code(), Some(0));
    let v = json(&o);
    assert_eq!(v["would_remove"]["runs"], 0);
    assert_eq!(v["kept"]["active"], 1);
    let o = p.gc(&["--older-than", "1h", "--state", "running"]);
    assert_eq!(
        o.status.code(),
        Some(2),
        "running cannot be named as a state to remove"
    );
    let args = p.apply_args(&["--older-than", "1h", "--state", "failed"]);
    assert!(args.windows(2).any(|w| w == ["--expect-runs", "0"]));
    let o = p.gc(&args);
    assert_eq!(o.status.code(), Some(0));
    assert!(p.run_exists(&run.id));
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_in_the_bundle_directory_is_left_alone() {
    let p = Project::new("link");
    p.old_run(RunState::Passed, Some("bundle l"));
    let outside = Project::new("link-outside");
    let real = p.bundles.join(format!(
        "svc-{}.hairspring",
        &hairspring_store::sha256_hex(b"bundle l")[..8]
    ));
    let target = outside.dir.join("precious");
    std::fs::write(&target, "bundle l").unwrap();
    std::fs::remove_file(&real).unwrap();
    std::os::unix::fs::symlink(&target, &real).unwrap();
    let o = p.gc(&p.apply_args(&["--older-than", "1h"]));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o).1);
    assert!(target.exists(), "the link was not followed");
    assert!(
        real.symlink_metadata().is_ok(),
        "the link itself is left alone"
    );
    assert!(text(&o).0.contains("symbolic link"), "{}", text(&o).0);
}

#[cfg(unix)]
#[test]
fn a_bundle_file_that_cannot_be_removed_is_reported_and_leaves_no_dangling_reference() {
    let p = Project::new("failure");
    let id = p.old_run(RunState::Passed, Some("bundle x"));
    std::fs::set_permissions(&p.bundles, perms(0o555)).unwrap();
    if std::fs::write(p.bundles.join("probe"), "").is_ok() {
        eprintln!("skipped: this user can write to a read-only directory");
        return;
    }
    let mut args = p.apply_args(&["--older-than", "1h"]);
    args.push("--json".into());
    let o = p.gc(&args);
    let (_, err) = text(&o);
    assert_eq!(o.status.code(), Some(2), "{err}");
    let v = json(&o);
    assert_eq!(v["removed"]["runs"], 1, "the store was committed first");
    assert_eq!(v["removed"]["bundle_files"], 0);
    assert_eq!(v["problems"].as_array().unwrap().len(), 1);
    assert!(err.contains("bundle file not removed"), "{err}");
    // Nothing in the store refers to the file that is left, and the run is gone, not half there.
    assert!(!p.run_exists(&id));
    let s = p.store();
    assert!(s.list_failures(&id).unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(&p.bundles).unwrap().count(),
        1,
        "the file stays for the operator"
    );
    // A second pass has nothing left to do and succeeds.
    let o = p.gc(&["--older-than", "1h"]);
    assert_eq!(o.status.code(), Some(0));
}

#[test]
fn a_removed_run_is_reported_as_not_stored_by_show() {
    let p = Project::new("show");
    let id = p.old_run(RunState::Passed, None);
    let o = p.gc(&p.apply_args(&["--older-than", "1h"]));
    assert_eq!(o.status.code(), Some(0));
    let shown = Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .arg("--db")
        .arg(&p.db)
        .args(["show", &id])
        .current_dir(&p.dir)
        .output()
        .unwrap();
    assert_eq!(shown.status.code(), Some(2));
    assert!(
        text(&shown).1.contains(&format!("no stored run `{id}`")),
        "{}",
        text(&shown).1
    );
}
