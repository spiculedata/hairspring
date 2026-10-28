//! Shared by the .NET conformance tests: find a .NET 10 SDK, or say there is none.
//!
//! `HS_DOTNET` names the `dotnet` command. Without it `$DOTNET_ROOT/dotnet` and then `dotnet` on `PATH` are tried.
//! `HS_REQUIRE_DOTNET` turns "none found" from a skip into an error.

use std::path::Path;
use std::process::{Command, Stdio};

/// The oldest .NET SDK major version the SDK is built with (it targets `net10.0`).
pub const MIN_DOTNET_MAJOR: u32 = 10;

/// Whether an environment switch is on: unset, empty, `0` and `false` (any case) are off, anything else is on.
pub fn switch_is_on(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        let v = v.trim();
        !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false"))
    })
}

/// Whether `HS_REQUIRE_DOTNET` asks for a missing or too old .NET SDK to be an error instead of a skip.
pub fn require_dotnet() -> bool {
    switch_is_on(std::env::var("HS_REQUIRE_DOTNET").ok().as_deref())
}

/// The major version in the output of `dotnet --version` (for example `10.0.400` or `10.0.100-rc.1.25451.107`), or None.
pub fn parse_major(version_output: &str) -> Option<u32> {
    version_output
        .trim()
        .split(['.', '-', '+'])
        .next()?
        .parse()
        .ok()
}

/// Whether this `dotnet --version` output is an SDK new enough for the SDK.
pub fn is_new_enough(version_output: &str) -> bool {
    parse_major(version_output).is_some_and(|major| major >= MIN_DOTNET_MAJOR)
}

/// Whether `command --version` runs and reports an SDK new enough. It runs from the temporary directory, outside the
/// repository, so the repository's `global.json` pin does not make an installed SDK look absent: a build with the wrong SDK
/// then fails with dotnet's own message ("Install the [10.0.402] .NET SDK or update global.json") instead of a quiet skip.
pub fn dotnet_usable(command: &str) -> bool {
    Command::new(command)
        .current_dir(std::env::temp_dir())
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|out| out.status.success() && is_new_enough(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or(false)
}

fn candidates() -> Vec<String> {
    let mut found = Vec::new();
    if let Some(root) = std::env::var_os("DOTNET_ROOT").filter(|v| !v.is_empty()) {
        found.push(
            Path::new(&root)
                .join("dotnet")
                .to_string_lossy()
                .into_owned(),
        );
    }
    found.push("dotnet".to_string());
    found
}

/// The `dotnet` command to use: `HS_DOTNET` if it is usable, else the first usable candidate. Panics when `HS_DOTNET` is
/// set but not usable (a typo should not read as a skip); None when nothing is available.
pub fn find_dotnet() -> Option<String> {
    if let Some(named) = std::env::var_os("HS_DOTNET").filter(|v| !v.is_empty()) {
        let named = named.to_string_lossy().into_owned();
        assert!(
            dotnet_usable(&named),
            "HS_DOTNET={named} is not a .NET SDK {MIN_DOTNET_MAJOR} or newer"
        );
        return Some(named);
    }
    candidates().into_iter().find(|c| dotnet_usable(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_major_version_of_a_release() {
        assert_eq!(parse_major("10.0.400\n"), Some(10));
        assert_eq!(parse_major("9.0.100"), Some(9));
    }

    #[test]
    fn reads_the_major_version_of_a_preview() {
        assert_eq!(parse_major("10.0.100-rc.1.25451.107"), Some(10));
        assert_eq!(parse_major("11.0.100+abc"), Some(11));
    }

    #[test]
    fn has_no_version_for_other_output() {
        assert_eq!(parse_major(""), None);
        assert_eq!(parse_major("not dotnet at all"), None);
    }

    #[test]
    fn rejects_an_sdk_older_than_the_sdk_supports() {
        assert!(!is_new_enough("8.0.404"));
        assert!(!is_new_enough("9.0.100"));
    }

    #[test]
    fn accepts_an_sdk_that_meets_the_requirement() {
        assert!(is_new_enough("10.0.0"));
        assert!(is_new_enough("11.0.100"));
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
        assert!(!dotnet_usable("/nonexistent/dotnet-for-hairspring-tests"));
    }
}
