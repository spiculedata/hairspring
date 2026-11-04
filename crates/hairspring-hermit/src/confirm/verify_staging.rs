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
