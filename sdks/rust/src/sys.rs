//! The few libc entry points the SDK needs, declared directly so the crate has no dependencies. The constants are
//! per platform; only Linux and macOS are supported.

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
compile_error!("hairspring-sdk supports Linux and macOS only");

use std::os::raw::c_int;

extern "C" {
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    pub fn atexit(callback: extern "C" fn()) -> c_int;
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    pub fn on_exit(
        callback: extern "C" fn(c_int, *mut std::os::raw::c_void),
        arg: *mut std::os::raw::c_void,
    ) -> c_int;
    pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    pub fn getuid() -> u32;
    pub fn geteuid() -> u32;
    pub fn getgid() -> u32;
    pub fn getegid() -> u32;
    pub fn pthread_sigmask(how: c_int, set: *const SigSet, old: *mut SigSet) -> c_int;
    pub fn sigemptyset(set: *mut SigSet) -> c_int;
    pub fn sigaddset(set: *mut SigSet, signal: c_int) -> c_int;
    pub fn sigismember(set: *const SigSet, signal: c_int) -> c_int;
    pub fn sigpending(set: *mut SigSet) -> c_int;
    pub fn sigwait(set: *const SigSet, signal: *mut c_int) -> c_int;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub fn getauxval(kind: std::os::raw::c_ulong) -> std::os::raw::c_ulong;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    pub fn issetugid() -> c_int;
}

pub const F_GETFD: c_int = 1;

/// Room for a `sigset_t` on any supported platform (128 bytes on Linux, 4 on macOS), aligned for either. The calls
/// above are the only things that look inside it.
#[repr(C, align(8))]
pub struct SigSet([u8; 128]);

impl SigSet {
    pub const fn zeroed() -> Self {
        Self([0; 128])
    }
}

pub const SIGPIPE: c_int = 13;
pub const EPIPE: i32 = 32;

#[cfg(any(target_os = "linux", target_os = "android"))]
pub const SIG_BLOCK: c_int = 0;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub const SIG_SETMASK: c_int = 2;
#[cfg(target_os = "macos")]
pub const SIG_BLOCK: c_int = 1;
#[cfg(target_os = "macos")]
pub const SIG_SETMASK: c_int = 3;

/// Opening a FIFO for writing must not wait for a reader.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub const O_NONBLOCK: c_int = 0o4000;
#[cfg(target_os = "macos")]
pub const O_NONBLOCK: c_int = 0x4;

#[cfg(any(target_os = "linux", target_os = "android"))]
pub const F_DUPFD_CLOEXEC: c_int = 1030;
#[cfg(target_os = "macos")]
pub const F_DUPFD_CLOEXEC: c_int = 67;

/// `open` refuses a symlink as the last path component.
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "powerpc",
        target_arch = "powerpc64"
    )
))]
pub const O_NOFOLLOW: c_int = 0o100_000;
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    not(any(
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "powerpc",
        target_arch = "powerpc64"
    ))
))]
pub const O_NOFOLLOW: c_int = 0o400_000;
#[cfg(target_os = "macos")]
pub const O_NOFOLLOW: c_int = 0x100;

/// The ids a process runs with.
#[derive(Clone, Copy, Debug)]
pub struct Ids {
    pub uid: u32,
    pub euid: u32,
    pub gid: u32,
    pub egid: u32,
}

/// Is the process set-uid or set-gid? `secure` is the kernel's or libc's own answer where there is one (the ELF
/// `AT_SECURE` flag, or `issetugid`); it is trusted when it says yes. The comparison of real and effective ids is
/// the fallback and is applied in every case, so the decision is never weaker than either.
pub fn setid_decision(secure: Option<bool>, ids: Ids) -> bool {
    secure == Some(true) || ids.uid != ids.euid || ids.gid != ids.egid
}

/// Is the process set-uid or set-gid? Then a path from the environment must not be trusted (the C SDK uses
/// `secure_getenv` or `issetugid` for the same reason).
pub fn is_setid() -> bool {
    // SAFETY: these calls take no arguments, touch no memory and cannot fail.
    let ids = unsafe {
        Ids {
            uid: getuid(),
            euid: geteuid(),
            gid: getgid(),
            egid: getegid(),
        }
    };
    setid_decision(secure_flag(), ids)
}

/// `AT_SECURE` on Linux: set by the kernel for a set-id or capability-gaining exec, even when the ids now match.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn secure_flag() -> Option<bool> {
    const AT_SECURE: std::os::raw::c_ulong = 23;
    // SAFETY: getauxval only reads the auxiliary vector.
    Some(unsafe { getauxval(AT_SECURE) } != 0)
}

/// `issetugid` on macOS and the BSDs.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn secure_flag() -> Option<bool> {
    // SAFETY: issetugid takes no arguments and cannot fail.
    Some(unsafe { issetugid() } != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAME: Ids = Ids {
        uid: 1000,
        euid: 1000,
        gid: 1000,
        egid: 1000,
    };

    #[test]
    fn the_secure_flag_alone_is_enough() {
        assert!(setid_decision(Some(true), SAME));
    }

    #[test]
    fn differing_effective_ids_are_enough_even_when_the_flag_says_no() {
        let uid_differs = Ids { euid: 0, ..SAME };
        let gid_differs = Ids { egid: 0, ..SAME };
        for ids in [uid_differs, gid_differs] {
            assert!(setid_decision(Some(false), ids));
            assert!(setid_decision(None, ids));
        }
    }

    #[test]
    fn matching_ids_and_no_flag_is_an_ordinary_process() {
        assert!(!setid_decision(Some(false), SAME));
        assert!(!setid_decision(None, SAME));
    }
}
