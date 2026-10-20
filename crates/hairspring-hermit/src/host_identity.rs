//! Reads the identity of the machine and inputs a run depends on, for recording in bundles.
//!
//! Every field is a hash or a short identifier. No host name, user name or absolute path is recorded: files are named
//! by what they are (`host.loader.libc`), never by where they were found. A source the host does not offer is left
//! out, so replay reports it as unavailable instead of comparing against an invented value.
use hairspring_core::environment::{sha256_file, EnvironmentIdentity};
use hairspring_core::point::encoding::sha256_hex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const BASE_ENV: &str = "minimal";
const LIBC_CANDIDATES: [&str; 5] = [
    "lib/x86_64-linux-gnu/libc.so.6",
    "usr/lib/x86_64-linux-gnu/libc.so.6",
    "lib64/libc.so.6",
    "usr/lib64/libc.so.6",
    "lib/libc.so.6",
];
const LD_SO_CANDIDATES: [&str; 4] = [
    "lib64/ld-linux-x86-64.so.2",
    "lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
    "usr/lib64/ld-linux-x86-64.so.2",
    "lib/ld-linux-x86-64.so.2",
];
const CONTAINER_MARKERS: [(&str, &str); 2] =
    [(".dockerenv", "docker"), ("run/.containerenv", "podman")];

/// Where each host fact is read from. [`HostSources::system`] is the running machine; tests root it in a temp directory.
#[derive(Clone, Debug)]
pub struct HostSources {
    osrelease: PathBuf,
    cpuinfo: PathBuf,
    ld_so_cache: PathBuf,
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
            libc: LIBC_CANDIDATES.iter().map(under).collect(),
            ld_so: LD_SO_CANDIDATES.iter().map(under).collect(),
            container_markers: CONTAINER_MARKERS
                .iter()
                .map(|(rel, name)| (root.join(rel), *name))
                .collect(),
        }
    }
}

/// What the target runs: the command, where it starts, and the environment the guest is given (already built).
pub struct GuestInputs<'a> {
    pub command: &'a [String],
    pub cwd: Option<&'a str>,
    pub env: &'a BTreeMap<String, String>,
}

/// The identity of the host and the run's inputs. `hermit_version` is whatever `hermit --version` said, if anything.
pub fn collect(
    sources: &HostSources,
    hermit_bin: &Path,
    hermit_version: Option<&str>,
    guest: &GuestInputs,
) -> EnvironmentIdentity {
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
        ("host.container", Some(container(sources))),
        ("hermit.path.sha256", hash_of(hermit_bin)),
        ("hermit.version", hermit_version.map(str::to_string)),
        ("env.base_env", Some(BASE_ENV.to_string())),
        ("env.guest.sha256", Some(guest_env_hash(guest.env))),
        ("target.exe.sha256", target_hash(guest)),
    ];
    fields
        .into_iter()
        .fold(EnvironmentIdentity::default(), |id, (name, value)| {
            let Some(value) = value else { return id };
            // A value the record format cannot hold is left out, like one the host did not offer.
            id.clone().with_field(name, value).unwrap_or(id)
        })
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

/// A hash of `KEY=VALUE` lines in order. Only the hash is kept: configured values may be secrets.
fn guest_env_hash(env: &BTreeMap<String, String>) -> String {
    let text: String = env.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    sha256_hex(text.as_bytes())
}

/// The program the guest starts, found as the guest would: a path is taken as given (relative to the working
/// directory), a bare name is looked up on the guest's pinned `PATH`.
fn target_hash(guest: &GuestInputs) -> Option<String> {
    let program = guest.command.first()?;
    if program.contains('/') {
        let path = Path::new(program);
        let full = match (path.is_absolute(), guest.cwd) {
            (false, Some(cwd)) => Path::new(cwd).join(path),
            _ => path.to_path_buf(),
        };
        return hash_of(&full);
    }
    let search = guest.env.get("PATH")?;
    search
        .split(':')
        .map(|dir| Path::new(dir).join(program))
        .find(|candidate| candidate.is_file())
        .and_then(|found| hash_of(&found))
}
