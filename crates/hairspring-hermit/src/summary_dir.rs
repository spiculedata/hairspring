//! A private directory for Hermit's summary files.
//!
//! The summaries are written by Hermit and read back by us, so the directory must not be one another local user can
//! plant a symlink in. It is created `0700` inside the configured base directory (usually the world-writable
//! `/dev/shm`), refused if it already exists as anything but our own private directory, and holds randomly named files.
use std::fs::DirBuilder;
use std::io::{self, Read};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

extern "C" {
    fn geteuid() -> u32;
    fn fchmodat(dirfd: i32, path: *const std::os::raw::c_char, mode: u32, flags: i32) -> i32;
}

/// `AT_FDCWD` and `AT_SYMLINK_NOFOLLOW`, whose values differ between Linux and macOS.
#[cfg(target_os = "linux")]
const AT_FDCWD: i32 = -100;
#[cfg(target_os = "linux")]
const AT_SYMLINK_NOFOLLOW: i32 = 0x100;
#[cfg(not(target_os = "linux"))]
const AT_FDCWD: i32 = -2;
#[cfg(not(target_os = "linux"))]
const AT_SYMLINK_NOFOLLOW: i32 = 0x20;

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

    /// Creates `<base>/<prefix>-<euid>-<pid>-<random>` with mode 0700. The name is new every time, so two of these in
    /// one process never share (or remove) one another's directory.
    pub fn create_unique(base: &Path, prefix: &str) -> io::Result<PrivateDir> {
        // SAFETY: see `create`.
        let uid = unsafe { geteuid() };
        let path = base.join(format!(
            "{prefix}-{uid}-{}-{:016x}",
            std::process::id(),
            random_u64()
        ));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(PrivateDir { path })
    }

    /// A fresh, unpredictable file path inside the directory.
    pub fn file_for(&self, seed: u64) -> PathBuf {
        self.path.join(format!("{seed}-{:016x}.json", random_u64()))
    }

    /// A fresh private directory (mode 0700) for one trial's scratch files, removed on drop. The name is a fixed-width
    /// random number and nothing else, so the path's length never depends on the seed.
    pub fn scratch_for(&self) -> io::Result<ScratchDir> {
        let path = self.path.join(format!("{:016x}.d", random_u64()));
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
        // Drop cannot report; the per-trial directories inside are removed (and failures reported) by their owners.
        let _ = remove_tree(&self.path);
    }
}

/// `chmod` that never follows a symlink at `path`: a link is left alone (the call fails, which is fine).
fn chmod_no_follow(path: &Path, mode: u32) {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return;
    };
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call.
    unsafe {
        fchmodat(AT_FDCWD, c_path.as_ptr(), mode, AT_SYMLINK_NOFOLLOW);
    }
}

/// A path that names the directory `handle` is open on, whatever `dir` now points at. Linux resolves paths through
/// `/dev/fd`; elsewhere (development machines, where Hermit does not run) the path is used as given.
#[cfg(target_os = "linux")]
fn through(_dir: &Path, handle: &std::fs::File) -> PathBuf {
    PathBuf::from(format!("/dev/fd/{}", handle.as_raw_fd()))
}

#[cfg(not(target_os = "linux"))]
fn through(dir: &Path, _handle: &std::fs::File) -> PathBuf {
    dir.to_path_buf()
}

/// Makes `dir` and everything below it owner-accessible, so a directory the guest left at mode 000 cannot stop the
/// removal. A guest process that outlives the trial (one that left the process group) can swap a path for a symlink
/// while this runs, so no mode is ever set through a path that could be one: the mode is set without following links,
/// and a directory is entered through an open handle that is checked to be the directory that was looked at.
fn make_accessible(dir: &Path) {
    let Ok(before) = std::fs::symlink_metadata(dir) else {
        return;
    };
    if !before.file_type().is_dir() {
        return;
    }
    chmod_no_follow(dir, 0o700);
    let Ok(handle) = std::fs::File::open(dir) else {
        return;
    };
    match handle.metadata() {
        Ok(opened) if (opened.dev(), opened.ino()) == (before.dev(), before.ino()) => {}
        _ => return,
    }
    let through_handle = through(dir, &handle);
    if let Ok(entries) = std::fs::read_dir(&through_handle) {
        for entry in entries.flatten() {
            make_accessible(&through_handle.join(entry.file_name()));
        }
    }
}

/// Removes a directory tree, whatever modes the guest gave its contents. A tree already gone is fine.
fn remove_tree(path: &Path) -> io::Result<()> {
    make_accessible(path);
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
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

    /// Removes the directory and says why if it could not be removed. Dropping does the same but cannot report.
    pub fn remove(self) -> io::Result<()> {
        remove_tree(&self.path)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = remove_tree(&self.path);
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
        let (a, c) = (d.scratch_for().unwrap(), d.scratch_for().unwrap());
        assert_ne!(a.path(), c.path());
        assert_eq!(std::fs::metadata(a.path()).unwrap().mode() & 0o777, 0o700);
        let gone = a.path().to_path_buf();
        drop(a);
        assert!(!gone.exists());
    }

    #[test]
    fn unique_private_directories_never_share_a_name() {
        let b = base("unique");
        let (a, c) = (
            PrivateDir::create_unique(&b, "trials").unwrap(),
            PrivateDir::create_unique(&b, "trials").unwrap(),
        );
        assert_ne!(a.path(), c.path());
        assert_eq!(std::fs::metadata(a.path()).unwrap().mode() & 0o777, 0o700);
        let gone = a.path().to_path_buf();
        drop(a);
        assert!(
            !gone.exists() && c.path().exists(),
            "only its own is removed"
        );
    }

    #[test]
    fn cleanup_does_not_follow_a_symlink_out_of_the_tree() {
        let b = base("symlink-out");
        let outside = b.join("outside");
        std::fs::create_dir_all(outside.join("inner")).unwrap();
        std::fs::write(outside.join("file"), "keep").unwrap();
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(outside.join("file"), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let d = PrivateDir::create(&b).unwrap();
        let s = d.scratch_for().unwrap();
        symlink(&outside, s.path().join("link-to-dir")).unwrap();
        symlink(outside.join("file"), s.path().join("link-to-file")).unwrap();
        s.remove().unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().mode() & 0o777;
        assert_eq!(mode(&outside), 0o755, "the directory behind the link");
        assert_eq!(
            mode(&outside.join("file")),
            0o644,
            "the file behind the link"
        );
        assert!(
            outside.join("file").exists(),
            "nothing behind the link removed"
        );
    }

    #[test]
    fn scratch_names_have_a_fixed_width() {
        let b = base("width");
        let d = PrivateDir::create(&b).unwrap();
        let names: Vec<_> = (0..20)
            .map(|_| d.scratch_for().unwrap().path().file_name().unwrap().len())
            .collect();
        assert!(names.iter().all(|n| *n == names[0]), "{names:?}");
    }

    fn not_root() -> bool {
        // SAFETY: see `PrivateDir::create`.
        unsafe { geteuid() != 0 }
    }

    #[test]
    fn a_subdirectory_the_guest_made_unreadable_is_removed_anyway() {
        if !not_root() {
            return;
        }
        let b = base("chmod000");
        let d = PrivateDir::create(&b).unwrap();
        let s = d.scratch_for().unwrap();
        let sub = s.path().join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("f"), "x").unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
        std::fs::set_permissions(s.path().join("a"), std::fs::Permissions::from_mode(0o000))
            .unwrap();
        let path = s.path().to_path_buf();
        s.remove().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn a_removal_that_fails_is_reported() {
        if !not_root() {
            return;
        }
        let b = base("failremove");
        let d = PrivateDir::create(&b).unwrap();
        let s = d.scratch_for().unwrap();
        let parent = s.path().parent().unwrap().to_path_buf();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();
        let err = s.remove().unwrap_err();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{err}");
    }

    #[test]
    fn a_scratch_directory_cannot_be_made_once_the_private_one_is_gone() {
        let b = base("gone");
        let d = PrivateDir::create(&b).unwrap();
        std::fs::remove_dir_all(d.path()).unwrap();
        assert!(d.scratch_for().is_err());
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
