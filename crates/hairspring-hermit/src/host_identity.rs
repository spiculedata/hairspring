//! Reads the identity of the machine and inputs a run depends on, for recording in bundles.
//!
//! Every field is a hash or a short identifier. No host name, user name or absolute path is recorded: files are named
//! by what they are (`host.loader.libc`), never by where they were found. A source the host does not offer is left
//! out, so replay reports it as unavailable instead of comparing against an invented value.
use hairspring_core::environment::{
    is_unsafe_display_char, sha256_file, EnvironmentIdentity, MAX_FIELD_VALUE_CHARS,
};
use hairspring_core::point::encoding::sha256_hex;
use hairspring_engine::hermit_args;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const LIBC_CANDIDATES: [&str; 9] = [
    "lib/x86_64-linux-gnu/libc.so.6",
    "usr/lib/x86_64-linux-gnu/libc.so.6",
    "lib/aarch64-linux-gnu/libc.so.6",
    "usr/lib/aarch64-linux-gnu/libc.so.6",
    "lib64/libc.so.6",
    "usr/lib64/libc.so.6",
    "lib/libc.so.6",
    "lib/libc.musl-x86_64.so.1",
    "lib/libc.musl-aarch64.so.1",
];
const LD_SO_CANDIDATES: [&str; 9] = [
    "lib64/ld-linux-x86-64.so.2",
    "lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
    "usr/lib64/ld-linux-x86-64.so.2",
    "lib/ld-linux-x86-64.so.2",
    "lib/ld-linux-aarch64.so.1",
    "lib/aarch64-linux-gnu/ld-linux-aarch64.so.1",
    "usr/lib/ld-linux-aarch64.so.1",
    "lib/ld-musl-x86_64.so.1",
    "lib/ld-musl-aarch64.so.1",
];
const CONTAINER_MARKERS: [(&str, &str); 2] =
    [(".dockerenv", "docker"), ("run/.containerenv", "podman")];
/// Said when `/etc/ld.so.preload` does not exist, so "no preload" is a recorded value rather than a gap.
const ABSENT: &str = "absent";
/// Variables whose values are recorded (as one hash). Anything else may be a secret, and a hash of a short secret can
/// be guessed offline, so other variables contribute their name only.
const ALLOWLISTED_VARS: [&str; 6] = [
    "PATH",
    "LANG",
    "LANGUAGE",
    "TZ",
    "PYTHONDONTWRITEBYTECODE",
    "PYTHONHASHSEED",
];
const ALLOWLISTED_PREFIXES: [&str; 1] = ["LC_"];

/// Where each host fact is read from. [`HostSources::system`] is the running machine; tests root it in a temp directory.
#[derive(Clone, Debug)]
pub struct HostSources {
    osrelease: PathBuf,
    cpuinfo: PathBuf,
    ld_so_cache: PathBuf,
    ld_so_preload: PathBuf,
    libc: Vec<PathBuf>,
    ld_so: Vec<PathBuf>,
    container_markers: Vec<(PathBuf, &'static str)>,
}

impl HostSources {
    pub fn system() -> Self {
        Self::rooted(Path::new("/"))
    }

    pub fn rooted(root: &Path) -> Self {
        let under = |rel: &&str| root.join(rel);
        HostSources {
            osrelease: root.join("proc/sys/kernel/osrelease"),
            cpuinfo: root.join("proc/cpuinfo"),
            ld_so_cache: root.join("etc/ld.so.cache"),
            ld_so_preload: root.join("etc/ld.so.preload"),
            libc: LIBC_CANDIDATES.iter().map(under).collect(),
            ld_so: LD_SO_CANDIDATES.iter().map(under).collect(),
            container_markers: CONTAINER_MARKERS
                .iter()
                .map(|(rel, name)| (root.join(rel), *name))
                .collect(),
        }
    }
}

/// What is known about the Hermit executable: the hash of its contents and what `--version` said, each if available.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HermitIdentity {
    pub sha256: Option<String>,
    pub version: Option<String>,
}

/// What the target runs: the command, where it starts, the environment the guest is given (already built), the names
/// of the extra variables the Hermit process itself is given, and the extra Hermit arguments.
pub struct GuestInputs<'a> {
    pub command: &'a [String],
    pub cwd: Option<&'a str>,
    pub env: &'a BTreeMap<String, String>,
    pub extra_env_names: &'a [String],
    pub hermit_args: &'a [String],
    /// The target's `pass_env` setting: whether the engine names its variables for the guest where it can.
    pub pass_env: bool,
}

/// The identity of the host and the run's inputs.
pub fn collect(
    sources: &HostSources,
    hermit: &HermitIdentity,
    guest: &GuestInputs,
) -> EnvironmentIdentity {
    let names = guest_names(guest);
    let fields: Vec<(&str, Option<String>)> = vec![
        ("host.kernel_release", read_trimmed(&sources.osrelease)),
        ("host.cpu_model", cpu_field(&sources.cpuinfo, "model name")),
        ("host.cpu_flags.sha256", cpu_flags_hash(&sources.cpuinfo)),
        (
            "host.loader.ld_so_cache.sha256",
            hash_of(&sources.ld_so_cache),
        ),
        ("host.loader.libc.sha256", first_hash(&sources.libc)),
        ("host.loader.ld_so.sha256", first_hash(&sources.ld_so)),
        (
            "host.loader.ld_so_preload.sha256",
            Some(hash_of(&sources.ld_so_preload).unwrap_or_else(|| ABSENT.to_string())),
        ),
        ("host.container", Some(container(sources))),
        ("hermit.path.sha256", hermit.sha256.clone()),
        ("hermit.version", hermit.version.clone()),
        ("env.base_env", Some(base_env_mode(guest.hermit_args))),
        ("env.guest.passed", Some(guest_passed(guest))),
        ("env.guest.names", names.listed()),
        ("env.guest.names.sha256", names.fallback_hash()),
        (
            "env.guest.allowlisted.sha256",
            Some(allowlisted_hash(guest.env)),
        ),
        ("target.exe.sha256", target_hash(guest)),
        ("target.command.sha256", command_hash(guest)),
        ("target.cwd.sha256", cwd_hash(guest)),
    ];
    fields
        .into_iter()
        .fold(EnvironmentIdentity::default(), |id, (name, value)| {
            let Some(value) = value else { return id };
            // A value the record format cannot hold is left out, like one the host did not offer.
            id.clone().with_field(name, value).unwrap_or(id)
        })
}

/// Which version of the list of variables the engine names for the guest was in force: `none` when it names none.
/// Bump the number when that list changes, so a bundle from either side reports a change instead of matching.
const PASSED_LIST_VERSION: u32 = 1;
const PASSED_NONE: &str = "none";

fn guest_passed(guest: &GuestInputs) -> String {
    if hermit_args::pass_through_active(guest.hermit_args, guest.pass_env) {
        format!("hairspring-{PASSED_LIST_VERSION}")
    } else {
        PASSED_NONE.to_string()
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn hash_of(path: &Path) -> Option<String> {
    sha256_file(path).ok()
}

fn first_hash(candidates: &[PathBuf]) -> Option<String> {
    candidates.iter().find_map(|p| hash_of(p))
}

/// The value of the first `key : value` line of `/proc/cpuinfo`, which every logical CPU repeats.
fn cpu_field(cpuinfo: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(cpuinfo).ok()?;
    text.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key).then(|| v.trim().to_string())
    })
}

/// A hash of the sorted CPU feature set: features change what Hermit can virtualise, but the list is too long to record.
fn cpu_flags_hash(cpuinfo: &Path) -> Option<String> {
    let flags = cpu_field(cpuinfo, "flags").or_else(|| cpu_field(cpuinfo, "Features"))?;
    let mut names: Vec<&str> = flags.split_whitespace().collect();
    names.sort_unstable();
    names.dedup();
    Some(sha256_hex(names.join(" ").as_bytes()))
}

fn container(sources: &HostSources) -> String {
    sources
        .container_markers
        .iter()
        .find(|(marker, _)| marker.exists())
        .map_or("none", |(_, name)| name)
        .to_string()
}

/// The sorted names of every variable the guest can see, or a hash of them when the list is too long to record.
struct GuestNames(String);

impl GuestNames {
    fn fits(&self) -> bool {
        self.0.chars().count() <= MAX_FIELD_VALUE_CHARS
            && !self.0.chars().any(is_unsafe_display_char)
    }

    fn listed(&self) -> Option<String> {
        self.fits().then(|| self.0.clone())
    }

    fn fallback_hash(&self) -> Option<String> {
        (!self.fits()).then(|| sha256_hex(self.0.as_bytes()))
    }
}

fn guest_names(guest: &GuestInputs) -> GuestNames {
    let mut names: Vec<&str> = guest
        .env
        .keys()
        .map(String::as_str)
        .chain(guest.extra_env_names.iter().map(String::as_str))
        .collect();
    names.sort_unstable();
    names.dedup();
    GuestNames(names.join(","))
}

fn is_allowlisted(name: &str) -> bool {
    ALLOWLISTED_VARS.contains(&name) || ALLOWLISTED_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// A hash of `NAME=value` lines for the allowlisted variables only.
fn allowlisted_hash(env: &BTreeMap<String, String>) -> String {
    let text: String = env
        .iter()
        .filter(|(name, _)| is_allowlisted(name))
        .map(|(k, v)| format!("{k}={v}\n"))
        .collect();
    sha256_hex(text.as_bytes())
}

/// The base-environment mode Hermit was asked for (see [`hermit_args::base_env_mode`]).
pub(crate) fn base_env_mode(args: &[String]) -> String {
    hermit_args::base_env_mode(args)
}

fn is_regular_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// The program the guest starts, found as the guest would: a path is taken as given (relative to the working
/// directory), a bare name is looked up on the guest's pinned `PATH`. Only a regular file is hashed, so a device or a
/// FIFO named as the command is never opened.
fn target_hash(guest: &GuestInputs) -> Option<String> {
    let found = resolved_program(guest)?;
    is_regular_file(&found).then(|| hash_of(&found)).flatten()
}

/// Where the program named by the command is, or `None` when it cannot be found.
fn resolved_program(guest: &GuestInputs) -> Option<PathBuf> {
    let program = guest.command.first()?;
    if program.contains('/') {
        let path = Path::new(program);
        return Some(match (path.is_absolute(), guest.cwd) {
            (false, Some(cwd)) => Path::new(cwd).join(path),
            _ => path.to_path_buf(),
        });
    }
    guest
        .env
        .get("PATH")?
        .split(':')
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| is_regular_file(candidate))
}

/// A hash of the command line with the program as resolved (so the install path is part of it) and every argument as
/// configured. Paths are not canonicalised: the text of a path is what a program can react to. Only the hash is
/// recorded, because arguments can carry secrets and paths can carry user names.
fn command_hash(guest: &GuestInputs) -> Option<String> {
    let program = guest.command.first()?;
    let resolved = resolved_program(guest)
        .map_or_else(|| program.clone(), |p| p.to_string_lossy().into_owned());
    let argv =
        std::iter::once(resolved.as_str()).chain(guest.command[1..].iter().map(String::as_str));
    Some(sha256_hex(&nul_terminated(argv)))
}

/// A hash of the directory the program starts in: the configured `cwd`, or the one the engine itself runs from when
/// none is set (which is what the program inherits). The path text is hashed, never stored.
fn cwd_hash(guest: &GuestInputs) -> Option<String> {
    let here = std::env::current_dir().ok();
    let effective = match (guest.cwd.map(Path::new), here) {
        (Some(cwd), Some(here)) => here.join(cwd),
        (Some(cwd), None) => cwd.to_path_buf(),
        (None, here) => here?,
    };
    Some(sha256_hex(effective.to_string_lossy().as_bytes()))
}

/// The items joined so that `a b` and `a`, `b` differ: each is followed by a NUL byte.
fn nul_terminated<'a>(items: impl Iterator<Item = &'a str>) -> Vec<u8> {
    items
        .flat_map(|s| s.bytes().chain(std::iter::once(0)))
        .collect()
}
