//! Where the pinned programs and the state directory are on this host, and the limits the run path works within.
use crate::vmname::Stamp;
use std::path::PathBuf;
use std::time::Duration;

/// Environment variable names. The first two are shared with the machine check.
pub const ENV_BHYVE: &str = "HAIRSPRING_DHYVE_BHYVE";
pub const ENV_BHYVECTL: &str = "HAIRSPRING_DHYVE_BHYVECTL";
pub const ENV_SUDO: &str = "HAIRSPRING_DHYVE_SUDO";
pub const ENV_LIB: &str = "HAIRSPRING_DHYVE_LIB";
pub const ENV_STATE: &str = "HAIRSPRING_DHYVE_STATE";
pub const ENV_KERNEL: &str = "HAIRSPRING_DHYVE_KERNEL";
pub const ENV_INITRD: &str = "HAIRSPRING_DHYVE_INITRD";
pub const ENV_STAMP: &str = "HAIRSPRING_DHYVE_STAMP";
pub const ENV_CORE: &str = "HAIRSPRING_DHYVE_CORE";
pub const ENV_PIN_BHYVE: &str = "HAIRSPRING_DHYVE_PIN_BHYVE";
pub const ENV_PIN_BHYVECTL: &str = "HAIRSPRING_DHYVE_PIN_BHYVECTL";
pub const ENV_PIN_LIB: &str = "HAIRSPRING_DHYVE_PIN_LIB";

/// The pinned absolute path of `env`, which clears the environment of the programs that run as root.
pub const ENV_PROGRAM: &str = "/usr/bin/env";
/// The pinned absolute paths of the other programs the backend itself names.
pub const SUDO_PROGRAM: &str = "/usr/local/bin/sudo";
pub const CPUSET_PROGRAM: &str = "/usr/bin/cpuset";
pub const KLDSTAT_PROGRAM: &str = "/sbin/kldstat";
/// The host cores a VM may be pinned to.
pub const ALLOWED_CORES: [u32; 3] = [1, 2, 3];
/// The core every VM is pinned to unless the settings say another.
pub const DEFAULT_CORE: u32 = 1;

/// SHA-256 pins for the programs and the library directory. A pin that is set is checked before the first spawn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pins {
    pub bhyve: Option<String>,
    pub bhyvectl: Option<String>,
    /// SHA-256 of the manifest of the library directory: one `<sha256>  <name>` line per file, sorted by name.
    pub lib_manifest: Option<String>,
}

/// Where everything this backend runs is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    /// The program that gives the two VM programs root; the backend itself runs unprivileged.
    pub sudo: PathBuf,
    pub env: PathBuf,
    pub bhyve: PathBuf,
    pub bhyvectl: PathBuf,
    /// The directory the two VM programs load their libraries from.
    pub lib_dir: PathBuf,
    /// Lock, stamp counter, checkpoints and per-trial files.
    pub state_dir: PathBuf,
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    /// `kldstat`, which says whether the `vmm` module is loaded.
    pub kldstat: PathBuf,
    pub cpuset: PathBuf,
    /// The host core every VM is pinned to; one of [`ALLOWED_CORES`].
    pub core: u32,
    /// The stamp of the session, when the settings give one. Run numbers then come from `r01` under it; with none, a
    /// stamp is taken from the counter for each prepare.
    pub session_stamp: Option<Stamp>,
    pub pins: Pins,
    /// Refuse a `bhyve` or `bhyvectl` that is a symbolic link.
    pub require_regular_files: bool,
}

/// A required setting that is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingSetting(pub &'static str);

impl std::fmt::Display for MissingSetting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not set or is not usable", self.0)
    }
}

impl std::error::Error for MissingSetting {}

impl HostConfig {
    /// Settings with every optional part at its default: the pinned programs, core 1, no stamp, no pins, and symbolic
    /// links refused.
    pub fn new(
        bhyve: PathBuf,
        bhyvectl: PathBuf,
        lib_dir: PathBuf,
        state_dir: PathBuf,
        kernel: PathBuf,
        initrd: PathBuf,
    ) -> HostConfig {
        HostConfig {
            sudo: PathBuf::from(SUDO_PROGRAM),
            env: PathBuf::from(ENV_PROGRAM),
            bhyve,
            bhyvectl,
            lib_dir,
            state_dir,
            kernel,
            initrd,
            kldstat: PathBuf::from(KLDSTAT_PROGRAM),
            cpuset: PathBuf::from(CPUSET_PROGRAM),
            core: DEFAULT_CORE,
            session_stamp: None,
            pins: Pins::default(),
            require_regular_files: true,
        }
    }

    /// Reads the settings from the environment. Every one but `sudo` has no default.
    pub fn from_env() -> Result<HostConfig, MissingSetting> {
        Self::from_lookup(|name| std::env::var_os(name).filter(|v| !v.is_empty()))
    }

    pub fn from_lookup<F: Fn(&str) -> Option<std::ffi::OsString>>(
        lookup: F,
    ) -> Result<HostConfig, MissingSetting> {
        let need = |name: &'static str| lookup(name).map(PathBuf::from).ok_or(MissingSetting(name));
        let text = |name: &'static str| lookup(name).and_then(|v| v.into_string().ok());
        let mut host = HostConfig::new(
            need(ENV_BHYVE)?,
            need(ENV_BHYVECTL)?,
            need(ENV_LIB)?,
            need(ENV_STATE)?,
            need(ENV_KERNEL)?,
            need(ENV_INITRD)?,
        );
        // The program that gives root is pinned. Only a build with the stand-in VM programs may be told another.
        if cfg!(feature = "fake-vmm") {
            if let Some(sudo) = lookup(ENV_SUDO) {
                host.sudo = PathBuf::from(sudo);
            }
            if let Some(kldstat) = lookup("HAIRSPRING_DHYVE_KLDSTAT") {
                host.kldstat = PathBuf::from(kldstat);
            }
            if let Some(cpuset) = lookup("HAIRSPRING_DHYVE_CPUSET") {
                host.cpuset = PathBuf::from(cpuset);
            }
            host.require_regular_files = false;
        }
        host.session_stamp = text(ENV_STAMP)
            .map(|t| Stamp::parse(&t).map_err(|_| MissingSetting(ENV_STAMP)))
            .transpose()?;
        if let Some(core) = text(ENV_CORE) {
            host.core = core
                .parse()
                .ok()
                .filter(|c| ALLOWED_CORES.contains(c))
                .ok_or(MissingSetting(ENV_CORE))?;
        }
        host.pins = Pins {
            bhyve: text(ENV_PIN_BHYVE),
            bhyvectl: text(ENV_PIN_BHYVECTL),
            lib_manifest: text(ENV_PIN_LIB),
        };
        Ok(host)
    }
}

/// The waits and bounds of the run path. Tests use shorter ones; [`Timings::standard`] is what a host session uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timings {
    /// How long bhyve has to connect to the control socket.
    pub connect_wait: Duration,
    /// The whole of a prepare, from the start of the first boot to the exit after the shutdown command.
    pub prepare_cap: Duration,
    /// How long the three checkpoint files may take to appear and settle.
    pub checkpoint_wait: Duration,
    pub checkpoint_read_every: Duration,
    pub checkpoint_stable_gap: Duration,
    /// How long after RESUME the first log line that shows it was taken may take.
    pub resume_limit: Duration,
    pub resume_poll: Duration,
    /// The wait between the shutdown command and closing the socket.
    pub shutdown_delay: Duration,
    /// How long a trial may run before it is cut off.
    pub trial_cap: Duration,
    /// How long a VM that was asked to go has to exit before its starter is killed.
    pub kill_grace: Duration,
    /// How often the log sizes and the VM are looked at.
    pub watch_every: Duration,
    /// How long a second run waits for the host.
    pub lock_wait: Duration,
    /// The longest a helper command (destroy, free space) may take.
    pub helper_cap: Duration,
}

impl Timings {
    pub fn standard() -> Timings {
        Timings {
            connect_wait: Duration::from_secs(30),
            prepare_cap: Duration::from_secs(420),
            checkpoint_wait: Duration::from_secs(120),
            checkpoint_read_every: Duration::from_secs(1),
            checkpoint_stable_gap: Duration::from_secs(2),
            resume_limit: Duration::from_secs(10),
            resume_poll: Duration::from_millis(100),
            shutdown_delay: Duration::from_secs(2),
            trial_cap: Duration::from_secs(100),
            kill_grace: Duration::from_secs(5),
            watch_every: Duration::from_millis(500),
            lock_wait: Duration::from_secs(60),
            helper_cap: Duration::from_secs(60),
        }
    }

    /// Short waits for tests that run against the fake.
    pub fn fast() -> Timings {
        Timings {
            connect_wait: Duration::from_secs(5),
            prepare_cap: Duration::from_secs(20),
            checkpoint_wait: Duration::from_secs(5),
            checkpoint_read_every: Duration::from_millis(20),
            checkpoint_stable_gap: Duration::from_millis(50),
            resume_limit: Duration::from_secs(2),
            resume_poll: Duration::from_millis(10),
            shutdown_delay: Duration::from_millis(50),
            trial_cap: Duration::from_secs(10),
            kill_grace: Duration::from_millis(500),
            watch_every: Duration::from_millis(20),
            lock_wait: Duration::from_millis(300),
            helper_cap: Duration::from_secs(10),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn lookup_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).map(std::ffi::OsString::from)
    }

    const ALL: [(&str, &str); 6] = [
        (ENV_BHYVE, "/w/bin/bhyve"),
        (ENV_BHYVECTL, "/w/bin/bhyvectl"),
        (ENV_LIB, "/w/lib"),
        (ENV_STATE, "/w/state"),
        (ENV_KERNEL, "/w/img/kernel"),
        (ENV_INITRD, "/w/img/initrd"),
    ];

    #[test]
    fn every_setting_but_sudo_is_required() {
        let host = HostConfig::from_lookup(lookup_of(&ALL)).unwrap();
        assert_eq!(host.sudo, PathBuf::from("/usr/local/bin/sudo"));
        assert_eq!(host.env, PathBuf::from("/usr/bin/env"));
        assert_eq!(host.lib_dir, PathBuf::from("/w/lib"));
        for (i, (name, _)) in ALL.iter().enumerate() {
            let rest: Vec<_> = ALL
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, p)| *p)
                .collect();
            let err = HostConfig::from_lookup(lookup_of(&rest)).unwrap_err();
            assert_eq!(err, MissingSetting(name));
            assert!(err.to_string().contains(name));
        }
    }

    #[test]
    fn the_optional_settings_are_read_and_checked() {
        let mut pairs = ALL.to_vec();
        pairs.extend([
            (ENV_STAMP, "20261201T0540Z"),
            (ENV_CORE, "2"),
            (ENV_PIN_BHYVE, "aa"),
            (ENV_PIN_LIB, "cc"),
        ]);
        let host = HostConfig::from_lookup(lookup_of(&pairs)).unwrap();
        assert_eq!(host.session_stamp.unwrap().as_str(), "20261201T0540Z");
        assert_eq!(host.core, 2);
        assert_eq!(host.pins.bhyve.as_deref(), Some("aa"));
        assert_eq!(host.pins.bhyvectl, None);
        for (name, bad) in [(ENV_STAMP, "nope"), (ENV_CORE, "0"), (ENV_CORE, "x")] {
            let mut pairs = ALL.to_vec();
            pairs.push((name, bad));
            assert_eq!(
                HostConfig::from_lookup(lookup_of(&pairs)).unwrap_err(),
                MissingSetting(name),
                "{name}={bad}"
            );
        }
        let defaults = HostConfig::from_lookup(lookup_of(&ALL)).unwrap();
        assert_eq!((defaults.core, defaults.session_stamp), (1, None));
        assert_eq!(defaults.cpuset, PathBuf::from("/usr/bin/cpuset"));
    }

    #[cfg(feature = "fake-vmm")]
    #[test]
    fn sudo_can_be_set_only_in_a_build_with_the_stand_in_programs() {
        let mut pairs = ALL.to_vec();
        pairs.push((ENV_SUDO, "/opt/sudo"));
        let host = HostConfig::from_lookup(lookup_of(&pairs)).unwrap();
        assert_eq!(host.sudo, PathBuf::from("/opt/sudo"));
    }

    #[test]
    fn the_fast_timings_are_shorter_than_the_standard_ones() {
        let (fast, std) = (Timings::fast(), Timings::standard());
        assert!(fast.connect_wait < std.connect_wait);
        assert!(fast.trial_cap < std.trial_cap);
        assert_eq!(std.connect_wait, Duration::from_secs(30));
    }
}
