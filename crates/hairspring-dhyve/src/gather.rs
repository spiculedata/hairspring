//! Reads the facts `assess` judges. Read-only: it reads files and asks `kldstat` and `sysctl` questions with a
//! deadline. It never loads or unloads a kernel module, never starts a virtual machine and never writes a setting.
//! Off FreeBSD on x86-64 it runs nothing and reads nothing.
use crate::facts::DhyveFacts;

/// Reads this machine once.
pub fn gather() -> DhyveFacts {
    #[cfg(target_os = "freebsd")]
    if std::env::consts::ARCH == "x86_64" {
        return freebsd::gather();
    }
    DhyveFacts::platform_only(std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(target_os = "freebsd")]
mod freebsd {
    use crate::bounded::{run_read_only, COMMAND_DEADLINE};
    use crate::commands::Commands;
    use crate::facts::{DhyveFacts, ModuleFact};
    use crate::read::{
        binary_fact, cpu_from_boot_messages, first_line, leftover_names, parse_whole_number,
    };

    const BOOT_MESSAGES: &str = "/var/run/dmesg.boot";
    const VMM_DEVICES: &str = "/dev/vmm";

    pub fn gather() -> DhyveFacts {
        DhyveFacts {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            os_release: sysctl("kern.osrelease").and_then(|t| first_line(&t)),
            cpu: std::fs::read_to_string(BOOT_MESSAGES)
                .map_or(crate::facts::CpuFact::Unknown, |t| {
                    cpu_from_boot_messages(&t)
                }),
            module: module(),
            bhyve: binary_fact(std::env::var_os("HAIRSPRING_DHYVE_BHYVE")),
            bhyvectl: binary_fact(std::env::var_os("HAIRSPRING_DHYVE_BHYVECTL")),
            leftover_vms: leftovers(),
            panic_on_nmi: sysctl("machdep.panic_on_nmi").and_then(|t| parse_whole_number(&t)),
        }
    }

    fn sysctl(name: &str) -> Option<String> {
        run_read_only(&Commands::sysctl(name), COMMAND_DEADLINE)
            .ok()
            .filter(|a| a.success)
            .map(|a| a.stdout)
    }

    fn module() -> ModuleFact {
        match run_read_only(&Commands::kldstat(), COMMAND_DEADLINE) {
            Ok(a) if a.success => ModuleFact::Loaded,
            Ok(_) => ModuleFact::NotLoaded,
            Err(e) => ModuleFact::Unanswered(format!("kldstat: {e}")),
        }
    }

    /// The directory is absent when no module is loaded, which is no leftovers; any other failure is unknown.
    fn leftovers() -> Option<Vec<String>> {
        match std::fs::read_dir(VMM_DEVICES) {
            Ok(entries) => Some(leftover_names(
                entries
                    .filter_map(Result::ok)
                    .filter_map(|e| e.file_name().into_string().ok()),
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Vec::new()),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_freebsd_nothing_beyond_the_platform_is_read() {
        if std::env::consts::OS == "freebsd" {
            return;
        }
        let facts = gather();
        assert_eq!(
            facts,
            DhyveFacts::platform_only(std::env::consts::OS, std::env::consts::ARCH)
        );
    }
}
