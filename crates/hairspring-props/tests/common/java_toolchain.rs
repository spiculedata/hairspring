//! Shared by the Java conformance tests: find a JDK 21 or newer, or say there is none.
//!
//! `HS_JAVA` names the `java` command (its `javac` is looked for next to it, or named by `HS_JAVAC`). Without it
//! `$JAVA_HOME/bin/java` and then `java` on `PATH` are tried. `HS_REQUIRE_JAVA` turns "none found" from a skip into an
//! error.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The oldest Java major version the SDK supports (it is compiled with `--release 21`).
pub const MIN_JAVA_MAJOR: u32 = 21;

/// Whether an environment switch is on: unset, empty, `0` and `false` (any case) are off, anything else is on.
pub fn switch_is_on(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        let v = v.trim();
        !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false"))
    })
}

/// Whether `HS_REQUIRE_JAVA` asks for a missing or too old JDK to be an error instead of a skip.
pub fn require_java() -> bool {
    switch_is_on(std::env::var("HS_REQUIRE_JAVA").ok().as_deref())
}

/// The major version in the output of `java -version` (for example `openjdk version "21.0.5" 2024-10-15 LTS`, or
/// `java version "1.8.0_402"`, which is 8), or None.
pub fn parse_major(version_output: &str) -> Option<u32> {
    let quoted = version_output.split('"').nth(1)?;
    let mut parts = quoted.split(['.', '_', '-', '+']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        return parts.next()?.parse().ok();
    }
    Some(first)
}

/// Whether this `java -version` output is a JDK new enough for the SDK.
pub fn is_new_enough(version_output: &str) -> bool {
    parse_major(version_output).is_some_and(|major| major >= MIN_JAVA_MAJOR)
}

/// Whether `command -version` runs and reports a Java new enough for the SDK (the version goes to standard error).
pub fn java_usable(command: &str) -> bool {
    Command::new(command)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .map(|out| out.status.success() && is_new_enough(&String::from_utf8_lossy(&out.stderr)))
        .unwrap_or(false)
}

fn candidates() -> Vec<String> {
    let mut found = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME").filter(|v| !v.is_empty()) {
        found.push(
            Path::new(&home)
                .join("bin/java")
                .to_string_lossy()
                .into_owned(),
        );
    }
    found.push("java".to_string());
    found
}

/// The `java` command to use: `HS_JAVA` if it is usable, else the first usable candidate. Panics when `HS_JAVA` is set but
/// not usable (a typo should not read as a skip); None when nothing is available.
pub fn find_java() -> Option<String> {
    if let Some(named) = std::env::var_os("HS_JAVA").filter(|v| !v.is_empty()) {
        let named = named.to_string_lossy().into_owned();
        assert!(
            java_usable(&named),
            "HS_JAVA={named} is not a Java {MIN_JAVA_MAJOR} or newer"
        );
        return Some(named);
    }
    candidates().into_iter().find(|c| java_usable(c))
}

/// The `javac` that goes with a `java` command: `HS_JAVAC`, else the one in the same directory, else `javac` on `PATH`.
pub fn javac_for(java: &str) -> PathBuf {
    if let Some(named) = std::env::var_os("HS_JAVAC").filter(|v| !v.is_empty()) {
        return PathBuf::from(named);
    }
    let sibling = Path::new(java).with_file_name("javac");
    if Path::new(java).components().count() > 1 && sibling.is_file() {
        return sibling;
    }
    PathBuf::from("javac")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_major_version_of_a_modern_release() {
        assert_eq!(
            parse_major("openjdk version \"21.0.5\" 2024-10-15 LTS\nOpenJDK Runtime Environment"),
            Some(21)
        );
        assert_eq!(parse_major("openjdk version \"25\" 2025-09-16"), Some(25));
        assert_eq!(
            parse_major("openjdk version \"22-ea\" 2024-03-19"),
            Some(22)
        );
    }

    #[test]
    fn reads_the_major_version_of_an_old_release() {
        assert_eq!(parse_major("java version \"1.8.0_402\""), Some(8));
    }

    #[test]
    fn has_no_version_for_other_output() {
        assert_eq!(parse_major(""), None);
        assert_eq!(parse_major("not java at all"), None);
        assert_eq!(parse_major("version \"x.y\""), None);
    }

    #[test]
    fn rejects_a_jdk_older_than_the_sdk_supports() {
        assert!(!is_new_enough("openjdk version \"17.0.9\""));
        assert!(!is_new_enough("java version \"1.8.0_402\""));
    }

    #[test]
    fn accepts_a_jdk_that_meets_the_requirement() {
        assert!(is_new_enough("openjdk version \"21.0.0\""));
        assert!(is_new_enough("openjdk version \"23.0.1\""));
    }

    #[test]
    fn an_unset_empty_zero_or_false_switch_is_off() {
        assert!(!switch_is_on(None));
        assert!(!switch_is_on(Some("")));
        assert!(!switch_is_on(Some("0")));
        assert!(!switch_is_on(Some("False")));
        assert!(!switch_is_on(Some(" 0 ")));
    }

    #[test]
    fn any_other_value_turns_a_switch_on() {
        assert!(switch_is_on(Some("1")));
        assert!(switch_is_on(Some("true")));
    }

    #[test]
    fn a_missing_command_is_not_usable() {
        assert!(!java_usable("/nonexistent/java-for-hairspring-tests"));
    }
}
