//! Read-only checks of the machine the Hermit backend runs on. The backend's `probe()` is built from these.
pub mod kernel;
pub mod loader;
pub mod process_backend;
pub mod process_probe;

pub use loader::LoaderHashes;

use hairspring_engine::probe::ProbeReport;
use std::path::Path;

/// Reads this machine and judges whether the Hermit backend can run on it. Read-only and bounded: the one program it
/// starts, `hermit --version`, is given a short deadline.
pub fn probe_machine() -> ProbeReport {
    let facts = process_probe::probe();
    process_backend::probe_report(&process_backend::assess(&facts))
}

/// The file's text with surrounding space removed, or `None` when it cannot be read or is empty.
pub fn read_trimmed(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A value that differs between calls, for naming test scratch directories.
#[cfg(test)]
pub(crate) fn unique_stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_probe_names_every_prerequisite_whatever_the_machine() {
        let report = probe_machine();
        let labels: Vec<&str> = report.rows.iter().map(|r| r.label.as_str()).collect();
        for expected in ["platform", "kernel", "hermit", "perf counters", "ptrace"] {
            assert!(labels.contains(&expected), "{labels:?}");
        }
        assert_eq!(report.subject, "process backend");
        // A machine that cannot run it says why; one that can has nothing missing.
        assert_eq!(report.is_ready(), report.missing.is_empty());
    }

    #[test]
    fn a_file_with_only_space_reads_as_nothing() {
        let dir = std::env::temp_dir().join(format!("hairspring-trim-{}", unique_stamp()));
        std::fs::create_dir_all(&dir).unwrap();
        let blank = dir.join("blank");
        std::fs::write(&blank, " \n").unwrap();
        let full = dir.join("full");
        std::fs::write(&full, " 6.17.0 \n").unwrap();
        assert_eq!(read_trimmed(&blank), None);
        assert_eq!(read_trimmed(&full).as_deref(), Some("6.17.0"));
        assert_eq!(read_trimmed(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
