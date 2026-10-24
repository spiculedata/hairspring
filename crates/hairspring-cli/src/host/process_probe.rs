//! Reads the facts `process_backend` judges. Read-only: it opens a counter and closes it again, reads files under
//! `/proc` and asks Hermit for its version. It never writes a setting.
// The counter probe exists only on Linux x86-64; elsewhere the errno classification below has no caller.
#![cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
use super::process_backend::{HermitFact, PerfFact, ProcessBackendFacts};
use super::{read_trimmed, LoaderHashes};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

const PERF_EVENT_PARANOID_FILE: &str = "/proc/sys/kernel/perf_event_paranoid";
const PTRACE_SCOPE_FILE: &str = "/proc/sys/kernel/yama/ptrace_scope";
const KERNEL_RELEASE_FILE: &str = "/proc/sys/kernel/osrelease";
/// Where `scripts/install-hermit.sh` puts Hermit when no prefix is given, below `$HOME`.
const DEFAULT_INSTALL_SUFFIX: &str = ".local/share/hairspring/hermit/bin/hermit";

/// How the backend finds Hermit: `HAIRSPRING_HERMIT`, else `hermit` on `PATH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HermitLookup {
    Resolved(PathBuf),
    Missing { installed_at: Option<PathBuf> },
}

pub fn resolve_hermit(
    env_value: Option<OsString>,
    path_var: Option<&OsStr>,
    default_install: Option<PathBuf>,
) -> HermitLookup {
    if let Some(named) = env_value.filter(|v| !v.is_empty()) {
        return HermitLookup::Resolved(PathBuf::from(named));
    }
    let on_path = path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("hermit"))
        .find(|candidate| is_executable_file(candidate));
    match on_path {
        Some(found) => HermitLookup::Resolved(found),
        None => HermitLookup::Missing {
            installed_at: default_install.filter(|p| is_executable_file(p)),
        },
    }
}

fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The default install location of Hermit below `home`.
pub fn default_install_path(home: Option<OsString>) -> Option<PathBuf> {
    home.filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(DEFAULT_INSTALL_SUFFIX))
}

pub fn parse_int(text: &str) -> Option<i32> {
    text.trim().parse().ok()
}

const EPERM: i32 = 1;
const ENOENT: i32 = 2;
const ENXIO: i32 = 6;
const EACCES: i32 = 13;
const ENODEV: i32 = 19;
const ENOSYS: i32 = 38;
const EOPNOTSUPP: i32 = 95;

/// Maps the errno of a failed `perf_event_open` to what it means for the backend.
pub fn classify_perf_errno(errno: i32) -> PerfFact {
    match errno {
        EPERM | EACCES => PerfFact::Denied,
        ENOENT | ENXIO | ENODEV | ENOSYS | EOPNOTSUPP => PerfFact::NoCounters,
        other => PerfFact::Failed(other),
    }
}

/// `docker` or `podman` when the usual marker files exist below `root`.
pub fn detect_container(root: &Path) -> Option<String> {
    let markers = [(".dockerenv", "docker"), ("run/.containerenv", "podman")];
    markers
        .iter()
        .find(|(file, _)| root.join(file).exists())
        .map(|(_, kind)| kind.to_string())
}

/// Opens a user-space hardware branch counter for this process and closes it again.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn probe_perf() -> PerfFact {
    const SYS_PERF_EVENT_OPEN: i64 = 298;
    const PERF_TYPE_HARDWARE: u32 = 0;
    const PERF_COUNT_HW_BRANCH_INSTRUCTIONS: u64 = 4;
    const PERF_ATTR_SIZE_VER0: u32 = 64;
    const DISABLED: u64 = 1 << 0;
    const EXCLUDE_KERNEL: u64 = 1 << 5;
    const EXCLUDE_HV: u64 = 1 << 6;

    #[repr(C)]
    struct PerfEventAttr {
        kind: u32,
        size: u32,
        config: u64,
        sample_period: u64,
        sample_type: u64,
        read_format: u64,
        flags: u64,
        wakeup_events: u32,
        bp_type: u32,
        config1: u64,
    }
    extern "C" {
        fn syscall(number: i64, ...) -> i64;
        fn close(fd: i32) -> i32;
    }

    let attr = PerfEventAttr {
        kind: PERF_TYPE_HARDWARE,
        size: PERF_ATTR_SIZE_VER0,
        config: PERF_COUNT_HW_BRANCH_INSTRUCTIONS,
        sample_period: 0,
        sample_type: 0,
        read_format: 0,
        flags: DISABLED | EXCLUDE_KERNEL | EXCLUDE_HV,
        wakeup_events: 0,
        bp_type: 0,
        config1: 0,
    };
    // SAFETY: `attr` is a valid, fully initialised perf_event_attr of the size it declares and outlives the call;
    // pid 0 and cpu -1 mean "this process, any CPU"; the descriptor is closed right away if one was returned.
    let fd = unsafe {
        syscall(
            SYS_PERF_EVENT_OPEN,
            &attr as *const PerfEventAttr,
            0i64,
            -1i64,
            -1i64,
            0i64,
        )
    };
    if fd >= 0 {
        // SAFETY: `fd` was just returned by the kernel and nothing else owns it.
        unsafe {
            close(fd as i32);
        }
        return PerfFact::Usable;
    }
    classify_perf_errno(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn probe_perf() -> PerfFact {
    PerfFact::NotProbed
}

fn effective_uid_is_root() -> bool {
    extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: `geteuid` takes no arguments and cannot fail.
    unsafe { geteuid() == 0 }
}

/// The first line `ldd --version` prints (glibc writes it to stdout, musl to stderr), which names the C library.
fn libc_identity() -> Option<String> {
    let output = std::process::Command::new("ldd")
        .arg("--version")
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    [output.stdout, output.stderr]
        .iter()
        .filter_map(|bytes| {
            String::from_utf8_lossy(bytes)
                .lines()
                .next()
                .map(str::to_string)
        })
        .map(|line| line.trim().to_string())
        .find(|line| !line.is_empty())
}

fn probe_hermit(lookup: HermitLookup) -> HermitFact {
    match lookup {
        HermitLookup::Missing { installed_at } => HermitFact::Missing {
            installed_at: installed_at.map(|p| p.display().to_string()),
        },
        HermitLookup::Resolved(path) => match hairspring_hermit::probe_hermit_version(&path) {
            Some(version) => HermitFact::Found {
                path: path.display().to_string(),
                version,
                sha256: hairspring_core::environment::sha256_file(&path).ok(),
            },
            None => HermitFact::NotRunnable {
                path: path.display().to_string(),
            },
        },
    }
}

pub fn probe() -> ProcessBackendFacts {
    let lookup = resolve_hermit(
        std::env::var_os(hairspring_hermit::HERMIT_ENV),
        std::env::var_os("PATH").as_deref(),
        default_install_path(std::env::var_os("HOME")),
    );
    ProcessBackendFacts {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        kernel_release: read_trimmed(Path::new(KERNEL_RELEASE_FILE)),
        hermit: probe_hermit(lookup),
        perf: probe_perf(),
        perf_event_paranoid: read_trimmed(Path::new(PERF_EVENT_PARANOID_FILE))
            .and_then(|t| parse_int(&t)),
        ptrace_scope: read_trimmed(Path::new(PTRACE_SCOPE_FILE)).and_then(|t| parse_int(&t)),
        is_root: effective_uid_is_root(),
        container: detect_container(Path::new("/")),
        libc: libc_identity(),
        loader: LoaderHashes::probe(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hairspring-probe-{name}-{}-{}",
            std::process::id(),
            hairspring_store::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn the_environment_variable_wins_even_when_the_file_is_missing() {
        let found = resolve_hermit(Some("/nowhere/hermit".into()), None, None);
        assert_eq!(
            found,
            HermitLookup::Resolved(PathBuf::from("/nowhere/hermit"))
        );
    }

    #[test]
    fn an_empty_environment_variable_is_treated_as_unset() {
        let found = resolve_hermit(Some("".into()), None, None);
        assert_eq!(found, HermitLookup::Missing { installed_at: None });
    }

    #[test]
    fn path_is_searched_for_an_executable_named_hermit() {
        let empty = scratch("path-empty");
        let with = scratch("path-with");
        let hermit = executable(&with, "hermit");
        let path = std::env::join_paths([&empty, &with]).unwrap();
        let found = resolve_hermit(None, Some(path.as_os_str()), None);
        assert_eq!(found, HermitLookup::Resolved(hermit));
    }

    #[test]
    fn a_hermit_that_is_not_executable_is_not_found_on_path() {
        let dir = scratch("path-noexec");
        let file = dir.join("hermit");
        std::fs::write(&file, "x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let path = std::env::join_paths([&dir]).unwrap();
        let found = resolve_hermit(None, Some(path.as_os_str()), None);
        assert_eq!(found, HermitLookup::Missing { installed_at: None });
    }

    #[test]
    fn the_install_prefix_copy_is_reported_but_not_resolved() {
        let dir = scratch("prefix");
        let installed = executable(&dir, "hermit");
        let found = resolve_hermit(None, None, Some(installed.clone()));
        assert_eq!(
            found,
            HermitLookup::Missing {
                installed_at: Some(installed)
            }
        );
        let absent = resolve_hermit(None, None, Some(dir.join("absent")));
        assert_eq!(absent, HermitLookup::Missing { installed_at: None });
    }

    #[test]
    fn the_default_install_path_is_below_home() {
        assert_eq!(
            default_install_path(Some("/home/u".into())),
            Some(PathBuf::from(
                "/home/u/.local/share/hairspring/hermit/bin/hermit"
            ))
        );
        assert_eq!(default_install_path(None), None);
        assert_eq!(default_install_path(Some("".into())), None);
    }

    #[test]
    fn kernel_settings_parse_as_signed_integers() {
        assert_eq!(parse_int("2\n"), Some(2));
        assert_eq!(parse_int("-1"), Some(-1));
        assert_eq!(parse_int(""), None);
        assert_eq!(parse_int("two"), None);
    }

    #[test]
    fn perf_errors_split_into_refused_and_absent() {
        assert_eq!(classify_perf_errno(13), PerfFact::Denied);
        assert_eq!(classify_perf_errno(1), PerfFact::Denied);
        for absent in [2, 19, 95, 38, 6] {
            assert_eq!(
                classify_perf_errno(absent),
                PerfFact::NoCounters,
                "{absent}"
            );
        }
        assert_eq!(classify_perf_errno(24), PerfFact::Failed(24));
    }

    #[test]
    fn container_marker_files_are_recognised() {
        let root = scratch("container");
        assert_eq!(detect_container(&root), None);
        std::fs::write(root.join(".dockerenv"), "").unwrap();
        assert_eq!(detect_container(&root).as_deref(), Some("docker"));
        let podman = scratch("container-podman");
        std::fs::create_dir_all(podman.join("run")).unwrap();
        std::fs::write(podman.join("run/.containerenv"), "").unwrap();
        assert_eq!(detect_container(&podman).as_deref(), Some("podman"));
    }

    #[test]
    fn probing_this_machine_names_its_platform_and_never_panics() {
        let facts = probe();
        assert_eq!(facts.os, std::env::consts::OS);
        assert_eq!(facts.arch, std::env::consts::ARCH);
        let off_linux = facts.os != "linux" || facts.arch != "x86_64";
        assert_eq!(off_linux, facts.perf == PerfFact::NotProbed);
    }
}
