//! Shared by the Python conformance tests: find a Python 3.12 or newer, or say there is none.
//!
//! `HS_PYTHON` names the interpreter. Without it the first usable of `python3.14`, `python3.13`, `python3.12`,
//! `python3` and `python` on `PATH` is used. `HS_REQUIRE_PYTHON` turns "none found" from a skip into an error.

use std::process::{Command, Stdio};

/// The oldest Python 3 minor version the SDK supports.
pub const MIN_PYTHON_MINOR: u32 = 12;

const CANDIDATES: [&str; 5] = [
    "python3.14",
    "python3.13",
    "python3.12",
    "python3",
    "python",
];

/// Whether an environment switch is on: unset, empty, `0` and `false` (any case) are off, anything else is on.
pub fn switch_is_on(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        let v = v.trim();
        !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false"))
    })
}

/// Whether `HS_REQUIRE_PYTHON` asks for a missing or too old Python to be an error instead of a skip.
pub fn require_python() -> bool {
    switch_is_on(std::env::var("HS_REQUIRE_PYTHON").ok().as_deref())
}

/// `(major, minor)` from the output of `python --version` (for example `Python 3.12.3`), or None.
pub fn parse_version(output: &str) -> Option<(u32, u32)> {
    let version = output.split_whitespace().nth(1)?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor_text = parts.next()?;
    let digits: String = minor_text
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((major, digits.parse().ok()?))
}

/// Whether this `python --version` output is a Python 3 new enough for the SDK.
pub fn is_new_enough(output: &str) -> bool {
    parse_version(output).is_some_and(|(major, minor)| major == 3 && minor >= MIN_PYTHON_MINOR)
}

/// Whether `command --version` runs and reports a Python new enough for the SDK.
pub fn python_usable(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|out| out.status.success() && is_new_enough(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or(false)
}

/// The interpreter to use: `HS_PYTHON` if it is usable, else the first usable candidate. Panics when `HS_PYTHON` is
/// set but not usable (a typo should not read as a skip); None when nothing is available.
pub fn find_python() -> Option<String> {
    if let Some(named) = std::env::var_os("HS_PYTHON").filter(|v| !v.is_empty()) {
        let named = named.to_string_lossy().into_owned();
        assert!(
            python_usable(&named),
            "HS_PYTHON={named} is not a Python 3.{MIN_PYTHON_MINOR} or newer"
        );
        return Some(named);
    }
    CANDIDATES
        .iter()
        .find(|c| python_usable(c))
        .map(|c| (*c).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_of_a_release() {
        assert_eq!(parse_version("Python 3.12.3"), Some((3, 12)));
        assert_eq!(parse_version("Python 3.14.0rc1"), Some((3, 14)));
        assert_eq!(parse_version("Python 3.9"), Some((3, 9)));
    }

    #[test]
    fn has_no_version_for_other_output() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("not python at all"), None);
        assert_eq!(parse_version("Python"), None);
    }

    #[test]
    fn rejects_a_python_older_than_the_sdk_supports() {
        assert!(!is_new_enough("Python 3.11.9"));
        assert!(!is_new_enough("Python 2.7.18"));
    }

    #[test]
    fn accepts_a_python_that_meets_the_requirement() {
        assert!(is_new_enough("Python 3.12.0"));
        assert!(is_new_enough("Python 3.14.1"));
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
        assert!(!python_usable("/nonexistent/python-for-hairspring-tests"));
    }
}
