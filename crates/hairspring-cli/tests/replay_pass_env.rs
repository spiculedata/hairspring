//! `hairspring replay` across the setting that makes the engine name its variables for the guest under
//! `--base-env=minimal`: a bundle recorded before it (no `env.guest.passed`) reads as `none`.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FAKE_HERMIT: &str = "#!/bin/sh\nshift\nseed=0\nwhile [ \"$1\" != \"--\" ]; do case \"$1\" in --sched-seed=*) seed=${1#--sched-seed=};; esac; shift; done\nshift\nFAKE_SCHED_SEED=$seed exec \"$@\"\n";
const TARGET: &str = "[target.svc]\nkind = \"process\"\ncommand = [\"/bin/sh\", \"-c\", \"[ $((FAKE_SCHED_SEED % 5)) -eq 3 ] && { echo 'HAIRSPRING_VIOLATION: lost update'; exit 1; }; echo ok\"]\n";
const BUDGET: &str = "[target.svc.budget]\nmax_trials = 20\n";
const PASSED_LINE: &str = "param.env.guest.passed=";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hairspring-rpe-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn project(hermit_args: &str, pass_env: Option<bool>) -> String {
    let mut text = TARGET.to_string();
    if !hermit_args.is_empty() {
        text += &format!("hermit_args = [{hermit_args}]\n");
    }
    if let Some(value) = pass_env {
        text += &format!("pass_env = {value}\n");
    }
    text + BUDGET
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

struct Recorded {
    dir: PathBuf,
    hermit: PathBuf,
    bundle: String,
}

fn record(name: &str, hermit_args: &str, pass_env: Option<bool>) -> Recorded {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch(name);
    let hermit = dir.join("fake-hermit");
    std::fs::write(&hermit, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&hermit, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("hairspring.toml"), project(hermit_args, pass_env)).unwrap();
    let o = run(&dir, &hermit, &["test", "svc", "--seed", "1"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let file = std::fs::read_dir(dir.join(".hairspring/bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let bundle = std::fs::read_to_string(file).unwrap();
    Recorded {
        dir,
        hermit,
        bundle,
    }
}

impl Recorded {
    /// The bundle as an engine from before the field existed wrote it.
    fn old(&self) -> String {
        self.bundle
            .lines()
            .filter(|l| !l.starts_with(PASSED_LINE))
            .map(|l| format!("{l}\n"))
            .collect()
    }

    fn set_project(&self, hermit_args: &str, pass_env: Option<bool>) {
        std::fs::write(
            self.dir.join("hairspring.toml"),
            project(hermit_args, pass_env),
        )
        .unwrap();
    }

    fn replay(&self, bundle: &str) -> Output {
        let path = self.dir.join("b.hairspring");
        std::fs::write(&path, bundle).unwrap();
        run(&self.dir, &self.hermit, &["replay", path.to_str().unwrap()])
    }
}

/// The recorded seed is changed to one that passes, so the replay misses.
fn passing_seed(bundle: &str) -> String {
    assert!(bundle.contains("param.sched_seed=3"), "{bundle}");
    bundle.replace("param.sched_seed=3", "param.sched_seed=1")
}

const MINIMAL: &str = "\"--base-env=minimal\"";

#[test]
fn a_new_bundle_names_what_was_passed() {
    let passing = record("rec-pass", MINIMAL, None);
    assert!(
        passing
            .bundle
            .contains("param.env.guest.passed=hairspring-1"),
        "{}",
        passing.bundle
    );
    let off = record("rec-off", MINIMAL, Some(false));
    assert!(
        off.bundle.contains("param.env.guest.passed=none"),
        "{}",
        off.bundle
    );
    let host = record("rec-host", "", None);
    assert!(
        host.bundle.contains("param.env.guest.passed=none"),
        "{}",
        host.bundle
    );
}

#[test]
fn an_old_minimal_bundle_under_the_new_default_misses_as_inconclusive_with_exit_3() {
    let r = record("old-minimal", MINIMAL, Some(false));
    r.set_project(MINIMAL, None);
    let o = r.replay(&passing_seed(&r.old()));
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    let out = text(&o);
    assert!(
        out.contains("env.guest.passed (recorded none, now hairspring-1)"),
        "{out}"
    );
    assert!(
        out.contains("set pass_env = false on the target to replay as recorded"),
        "{out}"
    );
    assert!(out.contains("does not show the bug is fixed"), "{out}");
}

#[test]
fn an_old_minimal_bundle_with_pass_env_off_is_an_ordinary_miss() {
    let r = record("old-minimal-off", MINIMAL, Some(false));
    let o = r.replay(&passing_seed(&r.old()));
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(!text(&o).contains("environment differs"), "{}", text(&o));
}

#[test]
fn an_old_host_bundle_is_unchanged() {
    let r = record("old-host", "", None);
    let miss = r.replay(&passing_seed(&r.old()));
    assert_eq!(miss.status.code(), Some(0), "{}", text(&miss));
    assert!(!text(&miss).contains("env.guest.passed"), "{}", text(&miss));
    let hit = r.replay(&r.old());
    assert_eq!(hit.status.code(), Some(1), "{}", text(&hit));
}

#[test]
fn a_new_bundle_replayed_with_pass_env_off_misses_as_inconclusive() {
    let r = record("new-off", MINIMAL, None);
    r.set_project(MINIMAL, Some(false));
    let o = r.replay(&passing_seed(&r.bundle));
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    assert!(
        text(&o).contains("env.guest.passed=hairspring-1"),
        "{}",
        text(&o)
    );
}

#[test]
fn a_hit_across_the_change_still_exits_1() {
    let r = record("hit", MINIMAL, Some(false));
    r.set_project(MINIMAL, None);
    let o = r.replay(&r.old());
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("env.guest.passed"), "{}", text(&o));
}
