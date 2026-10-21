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
}

pub const F_GETFD: c_int = 1;

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

/// Is the process set-uid or set-gid? Then a path from the environment must not be trusted (the C SDK uses
/// `secure_getenv` or `issetugid` for the same reason).
pub fn is_setid() -> bool {
    // SAFETY: these calls take no arguments, touch no memory and cannot fail.
    unsafe { getuid() != geteuid() || getgid() != getegid() }
}
