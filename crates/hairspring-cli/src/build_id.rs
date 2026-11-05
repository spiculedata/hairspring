//! The build identity that `hairspring --version` prints after the crate version: the short commit and its commit date,
//! for example `0.1.0 (f819c1a 2026-10-24)`, or `0.1.0 (unknown)` when the commit cannot be found.
//!
//! This file is compiled twice: by `build.rs`, which asks git (or the `HAIRSPRING_BUILD_COMMIT` and
//! `HAIRSPRING_BUILD_DATE` environment variables) and passes the result to the compiler, and by the library, so the rules
//! are unit tested. It must not use anything outside the standard library.

/// Printed in place of the build identity when the commit is not known.
pub const UNKNOWN: &str = "unknown";
/// How many hex digits of the commit are printed.
pub const SHORT_COMMIT_LEN: usize = 7;
const FULL_COMMIT_LEN: usize = 40;

/// The commit cut to `SHORT_COMMIT_LEN` lower-case hex digits, or None if it is not a git commit id: 7 to 40 hex digits.
pub fn short_commit(raw: &str) -> Option<String> {
    let commit = raw.trim();
    let is_hex = commit.chars().all(|c| c.is_ascii_hexdigit());
    if !is_hex || commit.len() < SHORT_COMMIT_LEN || commit.len() > FULL_COMMIT_LEN {
        return None;
    }
    Some(commit[..SHORT_COMMIT_LEN].to_ascii_lowercase())
}

/// The date if it is `YYYY-MM-DD` with a plausible month and day, as `git show -s --format=%cs` prints it.
pub fn commit_date(raw: &str) -> Option<String> {
    let date = raw.trim();
    let parts: Vec<&str> = date.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return None;
    };
    let all_digits = |s: &str, len: usize| s.len() == len && s.chars().all(|c| c.is_ascii_digit());
    if !all_digits(year, 4) || !all_digits(month, 2) || !all_digits(day, 2) {
        return None;
    }
    let month: u32 = month.parse().ok()?;
    let day: u32 = day.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(date.to_string())
}

/// The text inside the brackets: `<short commit> <date>`, `<short commit> unknown` when only the date is missing, and
/// `unknown` when the commit is missing or malformed (a date without a commit identifies nothing).
pub fn build_id(commit: Option<&str>, date: Option<&str>) -> String {
    let Some(commit) = commit.and_then(short_commit) else {
        return UNKNOWN.to_string();
    };
    let date = date
        .and_then(commit_date)
        .unwrap_or_else(|| UNKNOWN.to_string());
    format!("{commit} {date}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_commit_is_cut_to_seven_lower_case_hex_digits() {
        let full = "F819C1A342884D1A206D62057540D5D0C83D9A7B";
        assert_eq!(short_commit(full).as_deref(), Some("f819c1a"));
        assert_eq!(short_commit(" f819c1a\n").as_deref(), Some("f819c1a"));
    }

    #[test]
    fn a_value_that_is_not_a_commit_id_is_rejected() {
        for bad in [
            "",
            "f819c1",
            "not-a-sha",
            "f819c1a 2026",
            "g819c1a",
            &"a".repeat(41),
        ] {
            assert_eq!(short_commit(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_a_yyyy_mm_dd_date_is_accepted() {
        assert_eq!(commit_date("2026-10-24\n").as_deref(), Some("2026-10-24"));
        for bad in [
            "",
            "2026-10",
            "26-10-24",
            "2026-13-01",
            "2026-00-10",
            "2026-10-32",
            "2026-1-24",
            "2026-10-24T10",
        ] {
            assert_eq!(commit_date(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_build_id_is_the_short_commit_and_its_date() {
        let id = build_id(
            Some("f819c1a342884d1a206d62057540d5d0c83d9a7b"),
            Some("2026-10-24"),
        );
        assert_eq!(id, "f819c1a 2026-10-24");
    }

    #[test]
    fn a_missing_or_malformed_date_keeps_the_commit() {
        assert_eq!(build_id(Some("f819c1a"), None), "f819c1a unknown");
        assert_eq!(
            build_id(Some("f819c1a"), Some("yesterday")),
            "f819c1a unknown"
        );
    }

    #[test]
    fn without_a_commit_the_build_id_is_unknown_even_with_a_date() {
        assert_eq!(build_id(None, None), "unknown");
        assert_eq!(build_id(None, Some("2026-10-24")), "unknown");
        assert_eq!(build_id(Some("oops"), Some("2026-10-24")), "unknown");
    }

    #[test]
    fn the_version_keeps_the_crate_version_first_and_the_build_id_in_brackets() {
        let expected = format!(
            "{} ({})",
            env!("CARGO_PKG_VERSION"),
            env!("HAIRSPRING_BUILD_ID")
        );
        assert_eq!(crate::cli::VERSION, expected);
        let id = env!("HAIRSPRING_BUILD_ID");
        let (commit, date) = id.split_once(' ').unwrap_or((id, ""));
        assert!(
            id == UNKNOWN
                || (short_commit(commit).as_deref() == Some(commit)
                    && (date == UNKNOWN || commit_date(date).is_some())),
            "{id}"
        );
    }
}
