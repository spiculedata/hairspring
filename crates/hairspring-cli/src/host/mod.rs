//! Read-only checks of the machine a process target runs on. They only ever produce warnings: determinism does not
//! depend on them, so they never fail a command.
mod kernel;

use hairspring_engine::config::{Config, TargetKind};
use std::io::Write;
use std::path::Path;

/// Where Linux publishes the running kernel's release (the same text as `uname -r`).
const KERNEL_RELEASE_FILE: &str = "/proc/sys/kernel/osrelease";

/// What the checks look at, read once. Built with [`HostFacts::probe`] on a real host, or by hand in tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostFacts {
    /// The kernel release, when this host reports one.
    pub kernel_release: Option<String>,
}

impl HostFacts {
    /// Reads the facts from this machine. Anything that cannot be read is left out, never an error: on a host without
    /// them (another operating system) the matching check is silently skipped.
    pub fn probe() -> HostFacts {
        HostFacts {
            kernel_release: read_trimmed(Path::new(KERNEL_RELEASE_FILE)),
        }
    }
}

fn read_trimmed(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Every warning the facts call for, in a fixed order.
pub fn warnings(facts: &HostFacts) -> Vec<String> {
    facts
        .kernel_release
        .as_deref()
        .and_then(kernel::warning)
        .into_iter()
        .collect()
}

/// Whether any of the named targets (all configured targets when `names` is empty) is a process target, the only
/// kind that runs under Hermit and so the only one the host checks matter for.
pub fn involves_process_target(config: &Config, names: &[String]) -> bool {
    let is_process =
        |t: &hairspring_engine::config::TargetSpec| matches!(t.kind, TargetKind::Process { .. });
    match names.is_empty() {
        true => config.targets.values().any(is_process),
        false => names
            .iter()
            .filter_map(|n| config.targets.get(n))
            .any(is_process),
    }
}

/// Writes each warning as a `warning:` line.
pub fn write_warnings(out: &mut dyn Write, facts: &HostFacts) -> std::io::Result<()> {
    warnings(facts)
        .iter()
        .try_for_each(|w| writeln!(out, "warning: {w}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "[target.svc]\nkind = \"process\"\ncommand = [\"/bin/true\"]\n";

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn process_targets_are_found_among_the_named_or_all_targets() {
        let config = Config::parse(CONFIG).unwrap();
        assert!(involves_process_target(&config, &[]));
        assert!(involves_process_target(&config, &names(&["svc"])));
        assert!(!involves_process_target(&config, &names(&["other"])));
        assert!(!involves_process_target(&Config::default(), &[]));
    }

    #[test]
    fn warnings_are_written_one_line_each_and_nothing_when_there_are_none() {
        let mut out = Vec::new();
        write_warnings(&mut out, &facts("6.8.0-146-generic")).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(
            text.starts_with("warning: kernel 6.8.0-146-generic"),
            "{text}"
        );
        let mut quiet = Vec::new();
        write_warnings(&mut quiet, &facts("6.17.0-42-generic")).unwrap();
        assert!(quiet.is_empty());
    }

    fn facts(release: &str) -> HostFacts {
        HostFacts {
            kernel_release: Some(release.to_string()),
        }
    }

    #[test]
    fn an_old_kernel_produces_one_warning() {
        assert_eq!(warnings(&facts("6.8.0-146-generic")).len(), 1);
    }

    #[test]
    fn a_new_kernel_or_none_at_all_produces_none() {
        assert!(warnings(&facts("6.17.0-42-generic")).is_empty());
        assert!(warnings(&HostFacts::default()).is_empty());
    }
}
