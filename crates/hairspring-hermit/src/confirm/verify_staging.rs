//! The file Hermit stages for a strict verify's result.
//!
//! Hermit writes `.hermit-verify-summary-XXXXXX` in the directory it runs in (the target's working directory) and
//! removes it when it exits normally. A `SIGKILL` at the time limit, or an interrupt, skips that, so it stays next to
//! the target's own files. The engine removes the ones that appeared during a verify it had to kill.
//!
//! Only files this verify can have made are touched: regular files with the staging name that were not there before
//! the run. Another process's verify running in the same directory at the same time could lose its staging file the
//! same way; that makes its verify inconclusive, never a wrong verdict. When Hermit put the file in an `ignored/`
//! directory of an enclosing git work tree, it is not found here and stays.
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const STAGING_PREFIX: &str = ".hermit-verify-summary-";

/// The staging files a directory held before a verify started.
pub(super) struct Staging {
    dir: PathBuf,
    before: BTreeSet<OsString>,
}

fn staging_names(dir: &Path) -> BTreeSet<OsString> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name())
        .filter(|name| name.to_string_lossy().starts_with(STAGING_PREFIX))
        .collect()
}

fn remove_if_file(path: &Path) -> bool {
    let is_file = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file());
    is_file && std::fs::remove_file(path).is_ok()
}

impl Staging {
    /// Looks at `cwd`, or the engine's own working directory when the target names none.
    pub(super) fn watch(cwd: Option<&str>) -> Option<Staging> {
        let dir = match cwd {
            Some(dir) => PathBuf::from(dir),
            None => std::env::current_dir().ok()?,
        };
        let before = staging_names(&dir);
        Some(Staging { dir, before })
    }

    /// Removes the staging files that appeared since [`Staging::watch`] and returns the names it could not remove.
    pub(super) fn remove_new(&self) -> Vec<String> {
        staging_names(&self.dir)
            .difference(&self.before)
            .filter(|name| !remove_if_file(&self.dir.join(name)))
            .map(|name| name.to_string_lossy().into_owned())
            .collect()
    }
}

/// What the detail adds when a staging file could not be removed.
pub(super) fn left_behind(names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    format!(
        "; Hermit's staging file could not be removed from the working directory: {}",
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hs-staging-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn watch(dir: &Path) -> Staging {
        Staging::watch(Some(&dir.display().to_string())).unwrap()
    }

    #[test]
    fn removes_a_new_regular_file_and_keeps_an_old_one() {
        let dir = scratch("regular");
        std::fs::write(dir.join(".hermit-verify-summary-Old000"), "").unwrap();
        let staging = watch(&dir);
        std::fs::write(dir.join(".hermit-verify-summary-New000"), "").unwrap();
        std::fs::write(dir.join("evidence.txt"), "x").unwrap();
        assert!(staging.remove_new().is_empty());
        assert!(dir.join(".hermit-verify-summary-Old000").exists());
        assert!(!dir.join(".hermit-verify-summary-New000").exists());
        assert!(dir.join("evidence.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_a_directory_with_the_staging_name_alone() {
        let dir = scratch("directory");
        let staging = watch(&dir);
        let sub = dir.join(".hermit-verify-summary-Dir000");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("inner"), "x").unwrap();
        // Only regular files are removed; anything else with the name is left and named in the detail.
        assert_eq!(staging.remove_new(), [".hermit-verify-summary-Dir000"]);
        assert!(sub.join("inner").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_or_removes_a_symlink_with_the_staging_name() {
        let dir = scratch("symlink");
        let target = dir.join("evidence.txt");
        std::fs::write(&target, "x").unwrap();
        let staging = watch(&dir);
        std::os::unix::fs::symlink(&target, dir.join(".hermit-verify-summary-Link00")).unwrap();
        assert_eq!(staging.remove_new(), [".hermit-verify-summary-Link00"]);
        assert!(target.exists());
        assert!(std::fs::symlink_metadata(dir.join(".hermit-verify-summary-Link00")).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn names_a_file_it_cannot_remove() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("readonly");
        let staging = watch(&dir);
        std::fs::write(dir.join(".hermit-verify-summary-Stuck0"), "").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let can_still_write = std::fs::write(dir.join("probe"), "").is_ok();
        let stuck = staging.remove_new();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        if !can_still_write {
            assert_eq!(stuck, [".hermit-verify-summary-Stuck0"]);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_detail_names_what_stayed_and_is_empty_when_nothing_did() {
        assert_eq!(left_behind(&[]), "");
        let text = left_behind(&["a".to_string(), "b".to_string()]);
        assert!(text.ends_with("working directory: a, b"), "{text}");
    }
}
