//! Passes the build identity (short commit and commit date) to the compiler as `HAIRSPRING_BUILD_ID`, so that
//! `hairspring --version` can tell two builds of the same crate version apart.
//!
//! The commit comes from `HAIRSPRING_BUILD_COMMIT` when it is set (release builds set it), otherwise from git. The date
//! comes from `HAIRSPRING_BUILD_DATE` when it is set, otherwise from git's commit date for that commit. Neither uses the
//! wall clock, so the same commit always gives the same text. When git is absent, fails, or does not track this crate
//! (a source tarball, or a copy inside another repository), the identity is `unknown`. This script never fails the build.
//!
//! There is no marker for uncommitted changes: watching every source file for it would rerun this script on each edit,
//! and a marker that is only refreshed sometimes would be wrong more often than it helps.

#[path = "src/build_id.rs"]
#[allow(dead_code)]
mod build_id;

use std::path::{Path, PathBuf};
use std::process::Command;

const COMMIT_ENV: &str = "HAIRSPRING_BUILD_COMMIT";
const DATE_ENV: &str = "HAIRSPRING_BUILD_DATE";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/build_id.rs");
    println!("cargo:rerun-if-env-changed={COMMIT_ENV}");
    println!("cargo:rerun-if-env-changed={DATE_ENV}");

    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let tracked = is_tracked_by_git(&manifest_dir);
    if tracked {
        watch_head(&manifest_dir);
    }

    let commit = non_empty_env(COMMIT_ENV).or_else(|| {
        tracked
            .then(|| git(&manifest_dir, &["rev-parse", "HEAD"]))
            .flatten()
    });
    if let Some(raw) = commit.as_deref() {
        if build_id::short_commit(raw).is_none() {
            println!("cargo:warning=ignoring {COMMIT_ENV}/git commit {raw:?}: not a commit id; the build id is unknown");
        }
    }
    let date = non_empty_env(DATE_ENV).or_else(|| {
        let commit = commit.as_deref()?;
        git(
            &manifest_dir,
            &["show", "-s", "--format=%cs", commit.trim()],
        )
    });

    let id = build_id::build_id(commit.as_deref(), date.as_deref());
    println!("cargo:rustc-env=HAIRSPRING_BUILD_ID={id}");
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The trimmed standard output of a git command run in `dir`, or None if git is missing or the command fails.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Whether this crate's manifest is tracked by the git repository around it, so a copy of the source placed inside some
/// other repository does not report that repository's commit.
fn is_tracked_by_git(manifest_dir: &Path) -> bool {
    git(manifest_dir, &["ls-files", "--error-unmatch", "Cargo.toml"]).is_some()
}

/// Reruns this script when HEAD moves: a checkout, a commit, or a fetch that updates the branch HEAD names. Only paths that
/// exist are named, because cargo reruns on every build for a path that is missing.
fn watch_head(manifest_dir: &Path) {
    let resolve = |args: &[&str]| git(manifest_dir, args).map(|p| absolute(manifest_dir, &p));
    let (Some(git_dir), Some(common_dir)) = (
        resolve(&["rev-parse", "--git-dir"]),
        resolve(&["rev-parse", "--git-common-dir"]),
    ) else {
        return;
    };
    let head = git_dir.join("HEAD");
    watch_if_exists(&head);
    watch_if_exists(&common_dir.join("packed-refs"));
    let branch_ref = std::fs::read_to_string(&head)
        .ok()
        .and_then(|text| text.trim().strip_prefix("ref: ").map(str::to_string));
    if let Some(branch_ref) = branch_ref {
        watch_if_exists(&common_dir.join(branch_ref));
    }
}

fn absolute(base: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn watch_if_exists(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
