//! Shared by the Go conformance tests: find out whether the Go command on hand is usable for the example module
//! (`go 1.24` in its `go.mod`, built with `GOTOOLCHAIN=local`, so an older Go cannot build it).

use std::process::{Command, Stdio};

/// The oldest Go minor version (1.x) that can build the example module.
pub const MIN_GO_MINOR: u32 = 24;

/// The `1.x` minor version in the output of `go version` (for example `go version go1.24.5 darwin/arm64`), or
/// None when the output has no release version (a development build, or something that is not Go).
pub fn parse_go_minor(version_output: &str) -> Option<u32> {
    let release = version_output
        .split_whitespace()
        .find_map(|word| word.strip_prefix("go1."))?;
    let digits: String = release.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Whether a Go release with this `go version` output is new enough. A development build, whose version has no
/// release number, is given the benefit of the doubt.
pub fn is_new_enough(version_output: &str) -> bool {
    parse_go_minor(version_output).is_none_or(|minor| minor >= MIN_GO_MINOR)
}

/// Whether `command version` runs and reports a Go new enough for the example.
pub fn go_usable(command: &str) -> bool {
    Command::new(command)
        .arg("version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|out| out.status.success() && is_new_enough(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_minor_version_of_a_release() {
        assert_eq!(parse_go_minor("go version go1.24.5 darwin/arm64"), Some(24));
        assert_eq!(parse_go_minor("go version go1.9 linux/amd64"), Some(9));
        assert_eq!(parse_go_minor("go version go1.24rc1 linux/amd64"), Some(24));
    }

    #[test]
    fn has_no_minor_version_for_a_development_build_or_other_output() {
        assert_eq!(
            parse_go_minor("go version devel +abc123 Mon linux/amd64"),
            None
        );
        assert_eq!(parse_go_minor(""), None);
        assert_eq!(parse_go_minor("not go at all"), None);
    }

    #[test]
    fn rejects_a_go_older_than_the_module_requires() {
        assert!(!is_new_enough("go version go1.22.12 linux/amd64"));
        assert!(!is_new_enough("go version go1.23.12 linux/amd64"));
    }

    #[test]
    fn accepts_a_go_that_meets_the_requirement() {
        assert!(is_new_enough("go version go1.24.0 linux/amd64"));
        assert!(is_new_enough("go version go1.27.1 darwin/arm64"));
        assert!(is_new_enough("go version devel go1.25-abc Mon linux/amd64"));
        assert!(is_new_enough("go version devel +abc123 Mon linux/amd64"));
    }

    #[test]
    fn a_missing_command_is_not_usable() {
        assert!(!go_usable("/nonexistent/go-for-hairspring-tests"));
    }
}
