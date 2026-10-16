//! A private directory for Hermit's summary files.
//!
//! The summaries are written by Hermit and read back by us, so the directory must not be one another local user can
//! plant a symlink in. It is created `0700` inside the configured base directory (usually the world-writable
//! `/dev/shm`), refused if it already exists as anything but our own private directory, and holds randomly named files.
use std::fs::DirBuilder;
use std::io::{self, Read};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

extern "C" {
    fn geteuid() -> u32;
}

#[derive(Debug)]
pub struct PrivateDir {
    path: PathBuf,
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, msg)
}

impl PrivateDir {
    /// Creates `<base>/hairspring-<euid>-<pid>` with mode 0700.
    pub fn create(base: &Path) -> io::Result<PrivateDir> {
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        let uid = unsafe { geteuid() };
        let path = base.join(format!("hairspring-{uid}-{}", std::process::id()));
        match DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {}
            // A leftover from a previous process that had this pid: acceptable only if it is genuinely ours.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => verify_ours(&path, uid)?,
            Err(e) => return Err(e),
        }
        Ok(PrivateDir { path })
    }

    /// A fresh, unpredictable file path inside the directory.
    pub fn file_for(&self, seed: u64) -> PathBuf {
        self.path.join(format!("{seed}-{:016x}.json", random_u64()))
    }

    /// A fresh private directory (mode 0700, unpredictable name) for one trial's scratch files, removed on drop.
    pub fn scratch_for(&self, seed: u64) -> io::Result<ScratchDir> {
        let path = self.path.join(format!("{seed}-{:016x}.d", random_u64()));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(ScratchDir { path })
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// One trial's scratch directory inside a [`PrivateDir`].
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn verify_ours(path: &Path, uid: u32) -> io::Result<()> {
    // `symlink_metadata` does not follow a symlink, so a planted link is seen as a link.
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_dir() {
        return Err(invalid(format!(
            "{} exists and is not a directory",
            path.display()
        )));
    }
    if meta.uid() != uid {
        return Err(invalid(format!(
            "{} is owned by another user",
            path.display()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(invalid(format!(
            "{} is accessible to other users",
            path.display()
        )));
    }
    Ok(())
}

fn random_u64() -> u64 {
    let mut bytes = [0u8; 8];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok()
    {
        return u64::from_le_bytes(bytes);
    }
    // No entropy source: fall back to something unique within this process.
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
    NEXT.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn base(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "hairspring-pd-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn expected(base: &Path) -> PathBuf {
        // SAFETY: see `PrivateDir::create`.
        let uid = unsafe { geteuid() };
        base.join(format!("hairspring-{uid}-{}", std::process::id()))
    }

    #[test]
    fn creates_an_owner_only_directory_and_removes_it_on_drop() {
        let b = base("create");
        let path = {
            let d = PrivateDir::create(&b).unwrap();
            let mode = std::fs::metadata(d.path()).unwrap().mode() & 0o777;
            assert_eq!(mode, 0o700);
            d.path().to_path_buf()
        };
        assert!(!path.exists(), "removed when dropped");
    }

    #[test]
    fn a_planted_symlink_is_refused_not_followed() {
        let b = base("symlink");
        let elsewhere = base("elsewhere");
        symlink(&elsewhere, expected(&b)).unwrap();
        let err = PrivateDir::create(&b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{err}");
        assert!(
            std::fs::read_dir(&elsewhere).unwrap().next().is_none(),
            "nothing written through the link"
        );
    }

    #[test]
    fn a_directory_open_to_other_users_is_refused() {
        let b = base("open");
        std::fs::create_dir(expected(&b)).unwrap();
        std::fs::set_permissions(expected(&b), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(PrivateDir::create(&b).is_err());
    }

    #[test]
    fn our_own_leftover_private_directory_is_reused() {
        let b = base("reuse");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(expected(&b))
            .unwrap();
        assert!(PrivateDir::create(&b).is_ok());
    }

    #[test]
    fn scratch_directories_are_private_distinct_and_removed_on_drop() {
        let b = base("scratch");
        let d = PrivateDir::create(&b).unwrap();
        let (a, c) = (d.scratch_for(7).unwrap(), d.scratch_for(7).unwrap());
        assert_ne!(a.path(), c.path());
        assert_eq!(std::fs::metadata(a.path()).unwrap().mode() & 0o777, 0o700);
        let gone = a.path().to_path_buf();
        drop(a);
        assert!(!gone.exists());
    }

    #[test]
    fn file_names_are_unique_and_unpredictable() {
        let b = base("names");
        let d = PrivateDir::create(&b).unwrap();
        let a = d.file_for(7);
        let c = d.file_for(7);
        assert_ne!(a, c);
        assert!(a.starts_with(d.path()));
    }
}
