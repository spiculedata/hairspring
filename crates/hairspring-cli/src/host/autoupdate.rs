//! Whether the host updates packages by itself. A library update regenerates the dynamic-loader cache, which changes
//! the schedule of every run that executes the loader, so recorded runs stop replaying.
//!
//! Only apt's configuration is read (`APT::Periodic::Unattended-Upgrade` and `APT::Periodic::Enable`). The
//! `apt-daily-upgrade` systemd timer is not looked at: it is enabled by default on Ubuntu whether or not anything is
//! configured to upgrade, so it says nothing by itself.
use super::apt_conf;
use std::path::Path;

/// Where apt keeps its configuration fragments, and its main configuration file (read after the fragments).
pub const APT_CONF_DIR: &str = "/etc/apt/apt.conf.d";
pub const APT_CONF_FILE: &str = "/etc/apt/apt.conf";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AutoUpdates {
    /// Unattended upgrades are switched on.
    Enabled,
    /// apt is here and unattended upgrades are not switched on.
    Disabled,
    /// No apt configuration: not a Debian or Ubuntu host, so the check does not apply.
    #[default]
    NotPresent,
    /// The configuration exists but the setting could not be found because something could not be read.
    Unreadable,
    /// The setting is there, but its last value is not one this reader interprets (such as `always`).
    Unknown,
}

pub fn warning(state: AutoUpdates) -> Option<String> {
    (state == AutoUpdates::Enabled).then(|| {
        "automatic package updates are enabled. A library update regenerates the dynamic-loader cache and changes \
         schedules, so recorded runs may stop replaying. Disable unattended upgrades on hosts used for recorded runs, \
         or record the loader hashes in bundles."
            .to_string()
    })
}

/// Whether apt reads a file of this name from its configuration directory: it ignores names that start with a dot or
/// use characters outside letters, digits, `_`, `-` and `.`, and files whose extension is anything but `.conf` (which
/// covers `.disabled`, `.dpkg-old`, `.dpkg-dist`, `.ucf-*`, `.save`, `.bak`, `.orig` and a trailing `~`).
pub fn is_read_by_apt(name: &str) -> bool {
    let valid_chars = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if name.is_empty() || name.starts_with('.') || !valid_chars {
        return false;
    }
    match name.rsplit_once('.') {
        None => true,
        Some((_, extension)) => extension == "conf",
    }
}

/// Reads the state from the fragments in `dir`, then `main_file`, without changing anything. A file that cannot be
/// read is skipped; the result is [`AutoUpdates::Unreadable`] only if the setting is found nowhere else.
pub fn probe(dir: &Path, main_file: &Path) -> AutoUpdates {
    let (mut texts, mut skipped, mut present) = (Vec::new(), false, false);
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            present = true;
            let mut paths: Vec<_> = entries
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_str().is_some_and(is_read_by_apt))
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            paths.sort();
            collect(&paths, &mut texts, &mut skipped);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => (present, skipped) = (true, true),
    }
    if main_file.exists() {
        present = true;
        collect(&[main_file.to_path_buf()], &mut texts, &mut skipped);
    }
    match (present, apt_conf::evaluate(&texts)) {
        (false, _) => AutoUpdates::NotPresent,
        (true, Some(state)) => state,
        (true, None) if skipped => AutoUpdates::Unreadable,
        (true, None) => AutoUpdates::Disabled,
    }
}

fn collect(paths: &[std::path::PathBuf], texts: &mut Vec<String>, skipped: &mut bool) {
    for path in paths {
        match read_capped(path) {
            Ok(text) => texts.push(text),
            Err(_) => *skipped = true,
        }
    }
}

/// Most bytes read from one file; they are a few lines long.
const MAX_FRAGMENT_BYTES: u64 = 1 << 20;

fn read_capped(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_FRAGMENT_BYTES)
        .read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const ON: &str = "APT::Periodic::Unattended-Upgrade \"1\";\n";
    const OFF: &str = "APT::Periodic::Unattended-Upgrade \"0\";\n";

    #[test]
    fn only_an_enabled_host_gets_the_warning() {
        assert!(warning(AutoUpdates::Enabled)
            .unwrap()
            .contains("Disable unattended upgrades"));
        for quiet in [
            AutoUpdates::Disabled,
            AutoUpdates::NotPresent,
            AutoUpdates::Unreadable,
            AutoUpdates::Unknown,
        ] {
            assert!(warning(quiet).is_none(), "{quiet:?}");
        }
    }

    #[test]
    fn apt_reads_plain_and_conf_names_and_ignores_the_rest() {
        for read in [
            "20auto-upgrades",
            "50unattended-upgrades",
            "10periodic.conf",
            "99_local",
        ] {
            assert!(is_read_by_apt(read), "{read}");
        }
        for ignored in [
            "20auto-upgrades.disabled",
            "50unattended-upgrades.dpkg-old",
            "50unattended-upgrades.dpkg-dist",
            "50unattended-upgrades.ucf-old",
            "10periodic.save",
            "10periodic.bak",
            "10periodic.orig",
            "10periodic~",
            ".hidden",
            "bad name",
            "",
        ] {
            assert!(!is_read_by_apt(ignored), "{ignored}");
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hairspring-autoupdate-{name}-{}-{}",
            std::process::id(),
            hairspring_store::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
    }

    fn probe_dir(dir: &Path) -> AutoUpdates {
        probe(dir, &dir.join("no-main-file"))
    }

    #[test]
    fn probing_reads_fragments_in_name_order() {
        let dir = scratch("order");
        put(&dir, "20auto-upgrades", ON);
        assert_eq!(probe_dir(&dir), AutoUpdates::Enabled);
        put(&dir, "99local", OFF);
        assert_eq!(probe_dir(&dir), AutoUpdates::Disabled);
    }

    #[test]
    fn a_renamed_or_leftover_fragment_does_not_count() {
        let dir = scratch("renamed");
        put(&dir, "20auto-upgrades.disabled", ON);
        put(&dir, "20auto-upgrades.dpkg-old", ON);
        put(&dir, "20auto-upgrades~", ON);
        assert_eq!(probe_dir(&dir), AutoUpdates::Disabled);
    }

    #[test]
    fn the_main_file_is_read_after_the_fragments() {
        let dir = scratch("main");
        put(&dir, "20auto-upgrades", ON);
        put(&dir, "apt.conf", OFF);
        assert_eq!(probe(&dir, &dir.join("apt.conf")), AutoUpdates::Disabled);
        let only_main = scratch("only-main");
        put(&only_main, "apt.conf", ON);
        let missing_dir = only_main.join("absent.d");
        assert_eq!(
            probe(&missing_dir, &only_main.join("apt.conf")),
            AutoUpdates::Enabled
        );
    }

    #[test]
    fn no_apt_configuration_at_all_means_not_present() {
        let dir = scratch("absent").join("nothing-here");
        assert_eq!(probe_dir(&dir), AutoUpdates::NotPresent);
    }

    #[test]
    fn a_path_that_cannot_be_listed_is_unreadable() {
        let dir = scratch("file");
        let not_a_dir = dir.join("plain-file");
        std::fs::write(&not_a_dir, "x").unwrap();
        assert_eq!(probe_dir(&not_a_dir), AutoUpdates::Unreadable);
    }

    #[cfg(unix)]
    fn deny_reading(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // A privileged user reads it anyway, and then there is nothing to test.
        std::fs::read(path).is_err()
    }

    #[cfg(unix)]
    #[test]
    fn one_unreadable_fragment_is_skipped_and_the_setting_found_elsewhere_still_counts() {
        let dir = scratch("skip");
        put(&dir, "20auto-upgrades", ON);
        put(&dir, "30other", "Acquire::Retries \"3\";\n");
        if deny_reading(&dir.join("30other")) {
            assert_eq!(probe_dir(&dir), AutoUpdates::Enabled);
        }
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_is_reported_only_when_the_setting_is_found_nowhere() {
        let dir = scratch("nowhere");
        put(&dir, "30other", "Acquire::Retries \"3\";\n");
        if deny_reading(&dir.join("30other")) {
            assert_eq!(probe_dir(&dir), AutoUpdates::Unreadable);
        }
    }
}
