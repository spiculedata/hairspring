//! Whether the host updates packages by itself. A library update regenerates the dynamic-loader cache, which changes
//! the schedule of every run that executes the loader, so recorded runs stop replaying.
use std::path::Path;

/// Where apt keeps its configuration fragments.
pub const APT_CONF_DIR: &str = "/etc/apt/apt.conf.d";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AutoUpdates {
    /// Unattended upgrades are switched on.
    Enabled,
    /// apt is here and unattended upgrades are not switched on.
    Disabled,
    /// No apt configuration: not a Debian or Ubuntu host, so the check does not apply.
    #[default]
    NotPresent,
    /// The configuration exists but could not be read, so nothing is known.
    Unreadable,
}

pub fn warning(state: AutoUpdates) -> Option<String> {
    (state == AutoUpdates::Enabled).then(|| {
        "automatic package updates are enabled. A library update regenerates the dynamic-loader cache and changes \
         schedules, so recorded runs may stop replaying. Disable unattended upgrades on hosts used for recorded runs, \
         or record the loader hashes in bundles."
            .to_string()
    })
}

/// The state apt's configuration fragments describe, given their contents in the order apt reads them (the last
/// assignment wins). Only the flat `APT::Periodic::Unattended-Upgrade "N";` form is understood.
pub fn from_configs(configs: &[String]) -> AutoUpdates {
    let last = configs
        .iter()
        .flat_map(|text| text.lines())
        .filter_map(periodic_value)
        .next_back();
    match last {
        Some(days) if days > 0 => AutoUpdates::Enabled,
        _ => AutoUpdates::Disabled,
    }
}

const SETTING: &str = "APT::Periodic::Unattended-Upgrade";

/// The number in `APT::Periodic::Unattended-Upgrade "N";` on this line, ignoring comments.
fn periodic_value(line: &str) -> Option<u32> {
    let code = line.split("//").next().unwrap_or("").trim();
    let value = code.strip_prefix(SETTING)?.trim_start();
    let quoted = value.strip_prefix('"')?;
    quoted.split('"').next()?.trim().parse().ok()
}

/// Most bytes read from one fragment; they are a few lines long.
const MAX_FRAGMENT_BYTES: u64 = 1 << 20;

/// Reads the state from the fragments in `dir`, without changing anything.
pub fn probe(dir: &Path) -> AutoUpdates {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return AutoUpdates::NotPresent,
        Err(_) => return AutoUpdates::Unreadable,
    };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    let texts: Result<Vec<String>, std::io::Error> = paths.iter().map(|p| read_capped(p)).collect();
    match texts {
        Ok(texts) => from_configs(&texts),
        Err(_) => AutoUpdates::Unreadable,
    }
}

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

    fn configs(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn a_nonzero_periodic_setting_is_enabled() {
        let on = configs(&["APT::Periodic::Unattended-Upgrade \"1\";\n"]);
        assert_eq!(from_configs(&on), AutoUpdates::Enabled);
        let weekly = configs(&["APT::Periodic::Unattended-Upgrade \"7\";"]);
        assert_eq!(from_configs(&weekly), AutoUpdates::Enabled);
    }

    #[test]
    fn a_zero_or_missing_setting_is_disabled() {
        let off = configs(&["APT::Periodic::Unattended-Upgrade \"0\";\n"]);
        assert_eq!(from_configs(&off), AutoUpdates::Disabled);
        assert_eq!(
            from_configs(&configs(&["Acquire::Retries \"3\";"])),
            AutoUpdates::Disabled
        );
        assert_eq!(from_configs(&[]), AutoUpdates::Disabled);
    }

    #[test]
    fn the_last_assignment_wins_and_comments_are_ignored() {
        let later_off = configs(&[
            "APT::Periodic::Unattended-Upgrade \"1\";",
            "APT::Periodic::Unattended-Upgrade \"0\";",
        ]);
        assert_eq!(from_configs(&later_off), AutoUpdates::Disabled);
        let commented = configs(&[
            "// APT::Periodic::Unattended-Upgrade \"1\";\n# APT::Periodic::Unattended-Upgrade \"1\";",
        ]);
        assert_eq!(from_configs(&commented), AutoUpdates::Disabled);
        let trailing = configs(&["APT::Periodic::Unattended-Upgrade \"1\"; // on"]);
        assert_eq!(from_configs(&trailing), AutoUpdates::Enabled);
    }

    #[test]
    fn only_an_enabled_host_gets_the_warning() {
        assert!(warning(AutoUpdates::Enabled)
            .unwrap()
            .contains("Disable unattended upgrades"));
        for quiet in [
            AutoUpdates::Disabled,
            AutoUpdates::NotPresent,
            AutoUpdates::Unreadable,
        ] {
            assert!(warning(quiet).is_none(), "{quiet:?}");
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hairspring-autoupdate-{name}-{}-{}",
            std::process::id(),
            hairspring_store::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn probing_reads_fragments_in_name_order() {
        let dir = scratch("order");
        std::fs::write(
            dir.join("20auto-upgrades"),
            "APT::Periodic::Unattended-Upgrade \"1\";\n",
        )
        .unwrap();
        assert_eq!(probe(&dir), AutoUpdates::Enabled);
        std::fs::write(
            dir.join("99local"),
            "APT::Periodic::Unattended-Upgrade \"0\";\n",
        )
        .unwrap();
        assert_eq!(probe(&dir), AutoUpdates::Disabled);
    }

    #[test]
    fn a_directory_that_is_not_there_means_not_present() {
        let dir = scratch("absent").join("nothing-here");
        assert_eq!(probe(&dir), AutoUpdates::NotPresent);
    }

    #[test]
    fn a_path_that_cannot_be_listed_is_unreadable() {
        let dir = scratch("file");
        let not_a_dir = dir.join("plain-file");
        std::fs::write(&not_a_dir, "x").unwrap();
        assert_eq!(probe(&not_a_dir), AutoUpdates::Unreadable);
    }
}
