//! Tests of `build.rs` itself: a scratch crate made of the same `build.rs` and `src/build_id.rs`, in a scratch git
//! repository, built with cargo, printing the `HAIRSPRING_BUILD_ID` the script produced. Skipped when git is missing.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BUILD_RS: &str = include_str!("../build.rs");
const BUILD_ID_RS: &str = include_str!("../src/build_id.rs");
const MAIN_RS: &str = "fn main() { println!(\"{}\", env!(\"HAIRSPRING_BUILD_ID\")); }\n";
const MANIFEST: &str =
    "[package]\nname = \"build-id-probe\"\nversion = \"0.0.1\"\nedition = \"2021\"\n\n[workspace]\n";

struct Probe {
    root: PathBuf,
    repo: PathBuf,
    target: PathBuf,
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn has_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// git in the probe repository, isolated from the user's and the system's git configuration.
fn git(repo: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "probe")
        .env("GIT_AUTHOR_EMAIL", "probe@example.invalid")
        .env("GIT_COMMITTER_NAME", "probe")
        .env("GIT_COMMITTER_EMAIL", "probe@example.invalid")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("git runs");
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn probe(name: &str) -> Probe {
    let root = std::env::temp_dir().join(format!(
        "hairspring-build-script-{name}-{}-{}",
        std::process::id(),
        hairspring_store::now_ms()
    ));
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("Cargo.toml"), MANIFEST).unwrap();
    std::fs::write(repo.join("build.rs"), BUILD_RS).unwrap();
    std::fs::write(repo.join("src/build_id.rs"), BUILD_ID_RS).unwrap();
    std::fs::write(repo.join("src/main.rs"), MAIN_RS).unwrap();
    std::fs::write(repo.join(".gitignore"), "Cargo.lock\n").unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    Probe {
        target: root.join("target"),
        root,
        repo,
    }
}

/// Builds and runs the probe; returns the build id it prints and cargo's output.
fn build_id(p: &Probe, envs: &[(&str, &str)]) -> (String, Output) {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut cmd = Command::new(cargo);
    cmd.args(["run", "--offline"])
        .current_dir(&p.repo)
        .env("CARGO_TARGET_DIR", &p.target)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("HAIRSPRING_BUILD_COMMIT")
        .env_remove("HAIRSPRING_BUILD_DATE")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("cargo runs");
    assert!(
        o.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    (String::from_utf8_lossy(&o.stdout).trim().to_string(), o)
}

fn head_id(repo: &Path) -> String {
    let head = git(repo, &["rev-parse", "HEAD"]);
    let date = git(repo, &["show", "-s", "--format=%cs", "HEAD"]);
    format!("{} {date}", &head[..7])
}

#[test]
fn the_build_id_follows_new_commits_after_the_branch_ref_is_packed() {
    if !has_git() {
        eprintln!("skipped: git is not installed");
        return;
    }
    let p = probe("packed");
    assert_eq!(build_id(&p, &[]).0, head_id(&p.repo));
    // Packing removes the loose ref; the next build runs the script with only packed-refs to read.
    git(&p.repo, &["pack-refs", "--all"]);
    assert_eq!(build_id(&p, &[]).0, head_id(&p.repo));
    // A commit now writes a new loose ref, leaving HEAD and packed-refs as they were.
    git(&p.repo, &["commit", "-q", "--allow-empty", "-m", "second"]);
    assert_eq!(
        build_id(&p, &[]).0,
        head_id(&p.repo),
        "stale build id after a commit on a packed branch"
    );
}

#[test]
fn a_hostile_commit_override_reaches_no_git_option_and_does_not_break_the_build() {
    if !has_git() {
        eprintln!("skipped: git is not installed");
        return;
    }
    let p = probe("hostile");
    let planted = p.root.join("planted");
    let hostile = format!("--output={}", planted.display());
    let (id, out) = build_id(&p, &[("HAIRSPRING_BUILD_COMMIT", hostile.as_str())]);
    assert_eq!(id, "unknown");
    assert!(!planted.exists(), "git wrote a file named by the override");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not a commit id"), "no warning: {stderr}");
    // A malformed date override is ignored with the commit kept.
    let head = git(&p.repo, &["rev-parse", "HEAD"]);
    let (id, out) = build_id(
        &p,
        &[
            ("HAIRSPRING_BUILD_COMMIT", head.as_str()),
            ("HAIRSPRING_BUILD_DATE", "--output=x"),
        ],
    );
    assert_eq!(id, format!("{} unknown", &head[..7]));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not YYYY-MM-DD"), "no warning: {stderr}");
    assert!(!p.repo.join("x").exists());
}
