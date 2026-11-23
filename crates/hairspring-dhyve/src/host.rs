//! Where the pinned programs and the state directory are on this host, and the limits the run path works within.
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

/// The pinned absolute path of `env`, which clears the environment of the programs that run as root.
pub const ENV_PROGRAM: &str = "/usr/bin/env";

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
}

/// A required setting that is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingSetting(pub &'static str);

impl std::fmt::Display for MissingSetting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not set", self.0)
    }
}

impl std::error::Error for MissingSetting {}

impl HostConfig {
    /// Reads the settings from the environment. Every one but `sudo` has no default.
    pub fn from_env() -> Result<HostConfig, MissingSetting> {
        Self::from_lookup(|name| std::env::var_os(name).filter(|v| !v.is_empty()))
    }

    pub fn from_lookup<F: Fn(&str) -> Option<std::ffi::OsString>>(
        lookup: F,
    ) -> Result<HostConfig, MissingSetting> {
        let need = |name: &'static str| lookup(name).map(PathBuf::from).ok_or(MissingSetting(name));
        Ok(HostConfig {
            sudo: lookup(ENV_SUDO)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/usr/local/bin/sudo")),
            env: PathBuf::from(ENV_PROGRAM),
            bhyve: need(ENV_BHYVE)?,
            bhyvectl: need(ENV_BHYVECTL)?,
            lib_dir: need(ENV_LIB)?,
            state_dir: need(ENV_STATE)?,
            kernel: need(ENV_KERNEL)?,
            initrd: need(ENV_INITRD)?,
        })
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
    fn sudo_can_be_set() {
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
