//! The clean-up of a trial directory while a surviving guest swaps a directory entry for a symlink or a FIFO.
//!
//! The swap is made at a fixed step through the clean-up's own hook, so the test does not depend on timing: the
//! entry is replaced after it was checked and before it is changed, after its mode was set and before it is opened,
//! and after it was opened and before it is read.
use super::*;
use std::cell::Cell;
use std::os::unix::fs::{symlink, PermissionsExt};

extern "C" {
    fn mkfifo(path: *const std::os::raw::c_char, mode: u32) -> i32;
}

/// After the open, only Linux can keep reading the directory that was opened (through `/proc/self/fd`); elsewhere
/// the path is used as given, which is acceptable because Hermit does not run there.
#[cfg(target_os = "linux")]
const STEPS: &[Step] = &[Step::Checked, Step::ModeSet, Step::Opened];
#[cfg(not(target_os = "linux"))]
const STEPS: &[Step] = &[Step::Checked, Step::ModeSet];

fn base(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hairspring-race-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().mode() & 0o777
}

fn fifo(path: &Path) {
    let c_path = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call.
    assert_eq!(unsafe { mkfifo(c_path.as_ptr(), 0o600) }, 0);
}

/// A directory outside the trial directory, with a subdirectory and a file, all of mode 0755 / 0644.
fn outside(b: &Path) -> PathBuf {
    let out = b.join("outside");
    std::fs::create_dir_all(out.join("inner")).unwrap();
    std::fs::write(out.join("file"), "keep").unwrap();
    let modes = [
        (out.clone(), 0o755),
        (out.join("inner"), 0o755),
        (out.join("file"), 0o644),
    ];
    for (p, m) in modes {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
    }
    out
}

/// Cleans up a scratch directory holding `sub/`, replacing `sub` at step `at` using `swap`. Returns the scratch
/// directory's path (gone by then) and how many times the swap ran.
fn clean_up_with_swap(b: &Path, at: Step, swap: impl Fn(&Path)) -> (PathBuf, u32) {
    let d = PrivateDir::create(b).unwrap();
    let s = d.scratch_for().unwrap();
    std::fs::create_dir_all(s.path().join("sub")).unwrap();
    std::fs::write(s.path().join("sub").join("f"), "x").unwrap();
    let swapped = Cell::new(0);
    make_accessible_hooked(s.path(), &|step, path| {
        if step == at && path.file_name().is_some_and(|n| n == "sub") {
            std::fs::remove_dir_all(path).unwrap();
            swap(path);
            swapped.set(swapped.get() + 1);
        }
    });
    let path = s.path().to_path_buf();
    s.remove().unwrap();
    (path, swapped.get())
}

#[test]
fn a_directory_swapped_for_a_symlink_never_changes_or_removes_what_it_points_at() {
    for &at in STEPS {
        let b = base("symlink");
        let out = outside(&b);
        let (scratch, swaps) = clean_up_with_swap(&b, at, |p| symlink(&out, p).unwrap());
        assert_eq!(swaps, 1, "{at:?}: the swap ran once");
        assert!(!scratch.exists(), "{at:?}: the trial directory is gone");
        assert_eq!(mode(&out), 0o755, "{at:?}: the directory behind the link");
        assert_eq!(mode(&out.join("inner")), 0o755, "{at:?}: a subdirectory");
        assert_eq!(mode(&out.join("file")), 0o644, "{at:?}: the file");
        assert_eq!(
            std::fs::read_to_string(out.join("file")).unwrap(),
            "keep",
            "{at:?}: nothing behind the link was removed"
        );
    }
}

#[test]
fn a_directory_swapped_for_a_fifo_does_not_block_or_leave_the_trial_directory() {
    for &at in STEPS {
        let b = base("fifo");
        let (scratch, swaps) = clean_up_with_swap(&b, at, fifo);
        assert_eq!(swaps, 1, "{at:?}: the swap ran once");
        assert!(!scratch.exists(), "{at:?}: the trial directory is gone");
    }
}

#[test]
fn a_directory_swapped_for_a_symlink_to_the_trial_directorys_parent_leaves_the_parent_alone() {
    let b = base("parent");
    let out = outside(&b);
    let (_, swaps) = clean_up_with_swap(&b, Step::Checked, |p| symlink(&b, p).unwrap());
    assert_eq!(swaps, 1);
    assert!(b.exists() && out.join("file").exists());
}
