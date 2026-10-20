//! Shared by the Go conformance tests: find out whether the Go command on hand is usable for the example module
//! (`go 1.24` in its `go.mod`, built with `GOTOOLCHAIN=local`, so an older Go cannot build it).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Whether an environment switch is on: unset, empty, `0` and `false` (any case) are off, anything else is on.
pub fn switch_is_on(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        let v = v.trim();
        !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false"))
    })
}

/// Whether `HS_REQUIRE_GO` asks for a missing or too old Go to be an error instead of a skip.
pub fn require_go() -> bool {
    switch_is_on(std::env::var("HS_REQUIRE_GO").ok().as_deref())
}

/// A directory of prebuilt binaries named by the environment variable `var`: panics with a message naming the
/// variable and the problem when the directory or one of the binaries is missing.
pub fn check_prebuilt_dir(var: &str, dir: &Path, binaries: &[&str]) -> PathBuf {
    assert!(
        dir.is_dir(),
        "{var}={} is not a directory (it should hold {})",
        dir.display(),
        binaries.join(" and ")
    );
    for binary in binaries {
        assert!(
            dir.join(binary).is_file(),
            "{var}={} has no `{binary}` (it should hold {})",
            dir.display(),
            binaries.join(" and ")
        );
    }
    dir.to_path_buf()
}

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
    fn an_unset_empty_zero_or_false_switch_is_off() {
        assert!(!switch_is_on(None));
        assert!(!switch_is_on(Some("")));
        assert!(!switch_is_on(Some("0")));
        assert!(!switch_is_on(Some("false")));
        assert!(!switch_is_on(Some("FALSE")));
        assert!(!switch_is_on(Some(" 0 ")));
    }

    #[test]
    fn any_other_value_turns_a_switch_on() {
        assert!(switch_is_on(Some("1")));
        assert!(switch_is_on(Some("true")));
        assert!(switch_is_on(Some("yes")));
    }

    #[test]
    #[should_panic(expected = "HS_TEST_BUILD_DIR")]
    fn a_prebuilt_directory_that_does_not_exist_is_named_in_the_message() {
        check_prebuilt_dir(
            "HS_TEST_BUILD_DIR",
            Path::new("/nonexistent/prebuilt"),
            &["x"],
        );
    }

    #[test]
    #[should_panic(expected = "missing-binary")]
    fn a_prebuilt_directory_without_the_binaries_names_the_missing_one() {
        check_prebuilt_dir(
            "HS_TEST_BUILD_DIR",
            Path::new(env!("CARGO_MANIFEST_DIR")),
            &["missing-binary"],
        );
    }

    #[test]
    fn a_prebuilt_directory_with_the_binaries_is_returned() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(
            check_prebuilt_dir("HS_TEST_BUILD_DIR", dir, &["Cargo.toml"]),
            dir
        );
    }

    #[test]
    fn a_missing_command_is_not_usable() {
        assert!(!go_usable("/nonexistent/go-for-hairspring-tests"));
    }
}
