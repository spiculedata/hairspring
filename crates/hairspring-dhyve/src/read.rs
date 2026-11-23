//! The pure and file-only readers the FreeBSD gatherer is built from. They are separate so they run, and are tested, on
//! every operating system.
use crate::facts::{BinaryFact, CpuFact};
use std::ffi::OsString;
use std::path::Path;

/// The prefix of the names of virtual machines this backend will create. Only such names are reported as leftovers.
pub const VM_NAME_PREFIX: &str = "hs_";
/// The longest VM name reported; anything longer is not one of ours.
const MAX_VM_NAME: usize = 64;

/// Whether the boot messages' VT-x feature line names EPT. The line is `VT-x:` followed by comma-separated features.
pub fn cpu_from_boot_messages(text: &str) -> CpuFact {
    let Some(line) = text.lines().find(|l| l.trim_start().starts_with("VT-x:")) else {
        return CpuFact::Unknown;
    };
    let features = line.trim_start().trim_start_matches("VT-x:");
    match features.split(',').any(|f| f.trim() == "EPT") {
        true => CpuFact::Ept,
        false => CpuFact::NoEpt,
    }
}

/// The names among `entries` that look like this backend's own VMs: the prefix, then letters, digits and `_` only (the
/// names come from a directory anyone with access may write to, and are printed). Sorted.
pub fn leftover_names<I: IntoIterator<Item = String>>(entries: I) -> Vec<String> {
    let mut names: Vec<String> = entries
        .into_iter()
        .filter(|n| n.starts_with(VM_NAME_PREFIX) && n.len() <= MAX_VM_NAME)
        .filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .collect();
    names.sort();
    names
}

/// A whole number from a command's output, or `None` for anything else.
pub fn parse_whole_number(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}

/// The first line of a command's output with surrounding space removed, or `None` when it is empty.
pub fn first_line(text: &str) -> Option<String> {
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

/// What is at the path an environment variable names: nothing configured, a readable regular file (hashed), or
/// something else. Reading the file starts no process.
pub fn binary_fact(configured: Option<OsString>) -> BinaryFact {
    let Some(value) = configured.filter(|v| !v.is_empty()) else {
        return BinaryFact::NotConfigured;
    };
    let path = Path::new(&value);
    let shown = path.display().to_string();
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return BinaryFact::Unusable { path: shown };
    }
    BinaryFact::Found {
        sha256: hairspring_core::environment::sha256_file(path).ok(),
        path: shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vt_x_line_with_ept_is_ept() {
        let text = "CPU: Intel\n  VT-x: PAT,HLT,MTRR,PAUSE,EPT,UG,VPID\nnext\n";
        assert_eq!(cpu_from_boot_messages(text), CpuFact::Ept);
    }

    #[test]
    fn a_vt_x_line_without_ept_is_no_ept() {
        assert_eq!(
            cpu_from_boot_messages("  VT-x: HLT,PAUSE,UG\n"),
            CpuFact::NoEpt
        );
    }

    #[test]
    fn a_word_that_only_contains_ept_is_not_ept() {
        assert_eq!(
            cpu_from_boot_messages("  VT-x: HLT,EPTX,NEPT\n"),
            CpuFact::NoEpt
        );
    }

    #[test]
    fn no_vt_x_line_is_unknown() {
        assert_eq!(cpu_from_boot_messages(""), CpuFact::Unknown);
        assert_eq!(cpu_from_boot_messages("EPT\nAMD-V: x\n"), CpuFact::Unknown);
    }

    #[test]
    fn only_names_with_the_prefix_and_plain_characters_are_leftovers() {
        let entries = [
            "hs_2_1_0",
            "other",
            "hs_bad name",
            "hs_\u{1b}[31m",
            "hs_1_1_1",
            &format!("hs_{}", "a".repeat(80)),
        ]
        .map(String::from);
        assert_eq!(
            leftover_names(entries),
            vec!["hs_1_1_1".to_string(), "hs_2_1_0".to_string()]
        );
    }

    #[test]
    fn numbers_and_lines_are_read_strictly() {
        assert_eq!(parse_whole_number(" 255\n"), Some(255));
        assert_eq!(parse_whole_number("x"), None);
        assert_eq!(parse_whole_number(""), None);
        assert_eq!(
            first_line("15.0-RELEASE\nmore"),
            Some("15.0-RELEASE".into())
        );
        assert_eq!(first_line("  \n"), None);
        assert_eq!(first_line(""), None);
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("hairspring-dhyve-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_unset_or_empty_variable_is_not_configured() {
        assert_eq!(binary_fact(None), BinaryFact::NotConfigured);
        assert_eq!(
            binary_fact(Some(OsString::new())),
            BinaryFact::NotConfigured
        );
    }

    #[test]
    fn a_missing_path_or_a_directory_is_unusable() {
        let d = scratch("unusable");
        let missing = d.join("nope");
        assert!(matches!(
            binary_fact(Some(missing.into_os_string())),
            BinaryFact::Unusable { .. }
        ));
        assert!(matches!(
            binary_fact(Some(d.into_os_string())),
            BinaryFact::Unusable { .. }
        ));
    }

    #[test]
    fn a_regular_file_is_found_and_hashed() {
        let d = scratch("found");
        let f = d.join("bhyve");
        std::fs::write(&f, b"abc").unwrap();
        let BinaryFact::Found { path, sha256 } = binary_fact(Some(f.clone().into_os_string()))
        else {
            panic!("not found");
        };
        assert_eq!(path, f.display().to_string());
        assert_eq!(
            sha256.as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }
}
