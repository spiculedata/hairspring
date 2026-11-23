//! Opening a file the guest controls.
//!
//! The guest can leave anything at the path it was told to write: a symlink to a host file, a FIFO that blocks every
//! reader, a device. Checking the path first and opening it after leaves a window for a swap, so the file is opened
//! with `O_NOFOLLOW | O_NONBLOCK` and the *open handle* is then checked to be a regular file. `O_CLOEXEC` is already
//! what `std` sets. The flag values are per platform because the workspace has no `libc` dependency.
//!
//! The table is [`open_flags_for`], keyed on the operating system and architecture names that `std::env::consts`
//! reports, so every row can be tested on every host. A platform with no row fails closed: [`open_regular`] refuses
//! with `ErrorKind::Unsupported` and opens nothing, rather than opening with no flags, because without `O_NONBLOCK` a
//! FIFO blocks the reader and without `O_NOFOLLOW` a symlink to a regular host file would pass the handle check.
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// `O_NONBLOCK`, the same on the BSDs and macOS.
const BSD_NONBLOCK: i32 = 0x4;
/// `O_NOFOLLOW`, the same on FreeBSD, OpenBSD, NetBSD, DragonFly BSD and macOS.
const BSD_NOFOLLOW: i32 = 0x100;
const LINUX_NONBLOCK: i32 = 0o4000;
/// `O_NOFOLLOW` on Linux x86, x86-64 and riscv64.
const LINUX_NOFOLLOW_X86: i32 = 0o400000;
/// `O_NOFOLLOW` on Linux arm and aarch64.
const LINUX_NOFOLLOW_ARM: i32 = 0o100000;

/// The `O_NOFOLLOW | O_NONBLOCK` value for an operating system and architecture, or `None` when this table has no
/// row for it. Names are those of `std::env::consts::{OS, ARCH}`.
pub(crate) fn open_flags_for(os: &str, arch: &str) -> Option<i32> {
    match (os, arch) {
        ("linux", "x86" | "x86_64" | "riscv64") => Some(LINUX_NOFOLLOW_X86 | LINUX_NONBLOCK),
        ("linux", "arm" | "aarch64") => Some(LINUX_NOFOLLOW_ARM | LINUX_NONBLOCK),
        ("macos" | "freebsd" | "openbsd" | "netbsd" | "dragonfly", _) => {
            Some(BSD_NOFOLLOW | BSD_NONBLOCK)
        }
        _ => None,
    }
}

fn open_flags() -> Option<i32> {
    open_flags_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Opens `path` for reading if, and only if, it is a regular file reached without following a symlink.
///
/// Refuses with `ErrorKind::Unsupported` on a platform that has no flag row, without touching the path.
pub fn open_regular(path: &Path) -> io::Result<File> {
    let Some(flags) = open_flags() else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no safe open flags are known for this platform",
        ));
    };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)?;
    if !file.metadata()?.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::path::PathBuf;

    extern "C" {
        fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hairspring-so-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fifo(path: &std::path::Path) {
        let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
        assert_eq!(unsafe { mkfifo(c.as_ptr(), 0o600) }, 0);
    }

    #[test]
    fn a_regular_file_opens_and_reads() {
        let d = dir("regular");
        std::fs::write(d.join("f"), "hello").unwrap();
        let mut text = String::new();
        open_regular(&d.join("f"))
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "hello");
    }

    #[test]
    fn a_missing_file_is_not_found() {
        let d = dir("missing");
        let err = open_regular(&d.join("nope")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    /// A FIFO planted at the path is refused at once; a plain open for reading would wait for a writer forever. The
    /// open runs on a thread so a regression fails the test instead of hanging it. Only the refusal is asserted, not
    /// the error code, which differs between platforms.
    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let d = dir("fifo");
        fifo(&d.join("f"));
        let path = d.join("f");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(open_regular(&path).is_err());
        });
        let refused = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("opening a FIFO blocked");
        assert!(refused);
    }

    #[test]
    fn a_symlink_is_refused_even_to_a_regular_file() {
        let d = dir("symlink");
        std::fs::write(d.join("target"), "x").unwrap();
        std::os::unix::fs::symlink(d.join("target"), d.join("f")).unwrap();
        assert!(open_regular(&d.join("f")).is_err());
    }

    #[test]
    fn a_symlink_to_a_device_and_a_device_are_refused() {
        let d = dir("device");
        std::os::unix::fs::symlink("/dev/null", d.join("f")).unwrap();
        assert!(open_regular(&d.join("f")).is_err());
        let err = open_regular(std::path::Path::new("/dev/null")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{err}");
    }

    #[test]
    fn linux_rows_keep_their_values() {
        assert_eq!(open_flags_for("linux", "x86_64"), Some(0o404000));
        assert_eq!(open_flags_for("linux", "x86"), Some(0o404000));
        assert_eq!(open_flags_for("linux", "riscv64"), Some(0o404000));
        assert_eq!(open_flags_for("linux", "aarch64"), Some(0o104000));
        assert_eq!(open_flags_for("linux", "arm"), Some(0o104000));
    }

    #[test]
    fn macos_and_the_bsds_have_both_flags() {
        for os in ["macos", "freebsd", "openbsd", "netbsd", "dragonfly"] {
            for arch in ["x86_64", "aarch64"] {
                assert_eq!(open_flags_for(os, arch), Some(0x104), "{os} {arch}");
            }
        }
    }

    #[test]
    fn every_known_row_sets_both_flags() {
        for os in [
            "linux",
            "macos",
            "freebsd",
            "openbsd",
            "netbsd",
            "dragonfly",
        ] {
            for arch in ["x86", "x86_64", "riscv64", "arm", "aarch64"] {
                if let Some(flags) = open_flags_for(os, arch) {
                    assert_ne!(flags, 0, "{os} {arch}");
                    assert_eq!(flags.count_ones(), 2, "{os} {arch}: one bit per flag");
                }
            }
        }
    }

    /// An unlisted platform has no row, so it is refused rather than given flags of zero.
    #[test]
    fn an_unlisted_platform_has_no_flags() {
        assert_eq!(open_flags_for("linux", "powerpc64"), None);
        assert_eq!(open_flags_for("linux", "s390x"), None);
        assert_eq!(open_flags_for("solaris", "x86_64"), None);
        assert_eq!(open_flags_for("illumos", "x86_64"), None);
        assert_eq!(open_flags_for("windows", "x86_64"), None);
        assert_eq!(open_flags_for("", ""), None);
    }

    /// Every platform the tests run on in CI has a row, so `open_regular` is not silently refusing there.
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    ))]
    #[test]
    fn this_platform_has_a_row() {
        assert!(open_flags().is_some());
    }

    /// FreeBSD cannot be run from the development machine; this pins the real values there.
    #[cfg(target_os = "freebsd")]
    #[test]
    fn freebsd_opens_with_nofollow_and_nonblock() {
        assert_eq!(open_flags(), Some(0x100 | 0x4));
    }
}
