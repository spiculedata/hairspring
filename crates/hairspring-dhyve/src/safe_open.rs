//! Opening a file the guest or the VM program leaves in a directory anyone with access may write to.
//!
//! The serial log is created by the VM program running as root, in a directory the backend's user can also write. A
//! symbolic link planted at its path would redirect the read, and a FIFO would block it. So the file is opened with
//! `O_NOFOLLOW | O_NONBLOCK` and the open handle is then checked to be a regular file. The flag values are per platform
//! because the workspace has no `libc` dependency; a platform with no row fails closed. This follows the Hermit
//! backend's `safe_open`.
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const BSD_NONBLOCK: i32 = 0x4;
const BSD_NOFOLLOW: i32 = 0x100;
const LINUX_NONBLOCK: i32 = 0o4000;
const LINUX_NOFOLLOW_X86: i32 = 0o400000;
const LINUX_NOFOLLOW_ARM: i32 = 0o100000;

/// The `O_NOFOLLOW | O_NONBLOCK` value for an operating system and architecture, or `None` with no row for it.
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

/// Opens `path` for reading if, and only if, it is a regular file reached without following a symbolic link.
pub fn open_regular(path: &Path) -> io::Result<File> {
    let Some(flags) = open_flags_for(std::env::consts::OS, std::env::consts::ARCH) else {
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
    use std::time::Duration;

    extern "C" {
        fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hs-dhyve-so-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fifo(path: &Path) {
        let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
        assert_eq!(unsafe { mkfifo(c.as_ptr(), 0o600) }, 0);
    }

    fn refused_without_blocking(path: PathBuf) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(open_regular(&path).is_err());
        });
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the open blocked")
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
        assert_eq!(
            open_regular(&d.join("none")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let d = dir("fifo");
        fifo(&d.join("f"));
        assert!(refused_without_blocking(d.join("f")));
    }

    #[test]
    fn a_symbolic_link_is_refused_even_to_a_regular_file() {
        let d = dir("link");
        std::fs::write(d.join("real"), "x").unwrap();
        std::os::unix::fs::symlink(d.join("real"), d.join("f")).unwrap();
        assert!(refused_without_blocking(d.join("f")));
    }

    #[test]
    fn a_platform_with_no_row_has_no_flags() {
        assert_eq!(open_flags_for("plan9", "x86_64"), None);
        assert!(open_flags_for("freebsd", "x86_64").is_some());
        assert_eq!(open_flags_for("freebsd", "x86_64"), Some(0x104));
    }
}
