//! Opening a file the guest controls.
//!
//! The guest can leave anything at the path it was told to write: a symlink to a host file, a FIFO that blocks every
//! reader, a device. Checking the path first and opening it after leaves a window for a swap, so the file is opened
//! with `O_NOFOLLOW | O_NONBLOCK` and the *open handle* is then checked to be a regular file. `O_CLOEXEC` is already
//! what `std` sets. The flag values are per platform because the workspace has no `libc` dependency; on a platform not
//! listed here they are zero and only the handle check protects, which still refuses everything but a regular file.
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "riscv64")
))]
const OPEN_FLAGS: i32 = 0o400000 | 0o4000; // O_NOFOLLOW | O_NONBLOCK
#[cfg(all(target_os = "linux", any(target_arch = "arm", target_arch = "aarch64")))]
const OPEN_FLAGS: i32 = 0o100000 | 0o4000; // O_NOFOLLOW | O_NONBLOCK
#[cfg(target_os = "macos")]
const OPEN_FLAGS: i32 = 0x100 | 0x4; // O_NOFOLLOW | O_NONBLOCK
#[cfg(not(any(
    all(
        target_os = "linux",
        any(
            target_arch = "x86",
            target_arch = "x86_64",
            target_arch = "riscv64",
            target_arch = "arm",
            target_arch = "aarch64"
        )
    ),
    target_os = "macos"
)))]
const OPEN_FLAGS: i32 = 0;

/// Opens `path` for reading if, and only if, it is a regular file reached without following a symlink.
pub fn open_regular(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_FLAGS)
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

    /// A FIFO planted at the path is refused at once; a plain open for reading would wait for a writer forever.
    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let d = dir("fifo");
        fifo(&d.join("f"));
        let err = open_regular(&d.join("f")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{err}");
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
}
