//! Parameters of the read endpoints, validated in one place so REST and MCP accept exactly the same inputs.
//!
//! Callers pass the raw text they received. Every value is checked here and anything unexpected is refused with a
//! message that names the parameter, never with an echo of the value.
use crate::{Result, ServiceError};
use hairspring_props::{AssertKind, PropertyStatus};
use hairspring_store::TrialVerdict;

/// Rows returned when a caller gives no limit.
pub const DEFAULT_PAGE: u32 = 100;
/// Most property or trial rows in one page.
pub const MAX_PAGE: u32 = 500;
/// Most seeds in one page of a property's seed list.
pub const MAX_SEED_PAGE: u32 = 1000;
/// Longest prefix filter, in bytes (the longest property id).
pub const MAX_PREFIX_BYTES: usize = 128;
/// Shortest and longest schedule fingerprint filter, in characters (a Hermit fingerprint is 16 hex digits).
const MIN_FP_CHARS: usize = 16;
const MAX_FP_CHARS: usize = 64;
/// Most codes in one page of the seed strip. It equals [`SCAN_CAP`]: one request reads at most that many trials.
pub const MAX_STRIP_PAGE: u32 = 20_000;
/// Most trials one request reads before it hands back a cursor, so a filter that matches nothing cannot hold a
/// connection for the length of a very large run.
pub const SCAN_CAP: usize = 20_000;
/// Most points in a coverage series.
pub const MAX_COVERAGE_POINTS: usize = 2000;

fn invalid(what: &str) -> ServiceError {
    ServiceError::InvalidRequest(format!("`{what}` is not valid"))
}

/// A page size within `1..=max`.
pub(crate) fn page_size(limit: Option<u32>, default: u32, max: u32) -> usize {
    limit.unwrap_or(default).clamp(1, max) as usize
}

/// A cursor is a position and nothing else: a non-negative integer.
pub(crate) fn parse_cursor(cursor: Option<&str>) -> Result<Option<u64>> {
    match cursor {
        None => Ok(None),
        Some(c) if !c.is_empty() && c.len() <= 20 && c.bytes().all(|b| b.is_ascii_digit()) => {
            c.parse().map(Some).map_err(|_| invalid("cursor"))
        }
        Some(_) => Err(invalid("cursor")),
    }
}

pub(crate) fn parse_status(s: Option<&str>) -> Result<Option<PropertyStatus>> {
    s.map(|s| crate::properties::parse_status(s).ok_or_else(|| invalid("status")))
        .transpose()
}

pub(crate) fn parse_kind(s: Option<&str>) -> Result<Option<AssertKind>> {
    s.map(|s| AssertKind::from_wire(s).ok_or_else(|| invalid("kind")))
        .transpose()
}

pub(crate) fn parse_prefix(s: Option<&str>) -> Result<Option<String>> {
    match s {
        None => Ok(None),
        Some(q) if q.len() <= MAX_PREFIX_BYTES => Ok(Some(q.to_owned())),
        Some(_) => Err(invalid("q")),
    }
}

pub(crate) fn parse_verdict(s: Option<&str>) -> Result<Option<TrialVerdict>> {
    s.map(|s| TrialVerdict::parse(s).ok_or_else(|| invalid("verdict")))
        .transpose()
}

pub(crate) fn parse_bool(s: Option<&str>, what: &str) -> Result<Option<bool>> {
    match s {
        None => Ok(None),
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        Some(_) => Err(invalid(what)),
    }
}

pub(crate) fn parse_idx(s: Option<&str>, what: &str) -> Result<Option<u32>> {
    s.map(|s| {
        if !s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit()) {
            s.parse().map_err(|_| invalid(what))
        } else {
            Err(invalid(what))
        }
    })
    .transpose()
}

/// A seed: plain decimal digits that fit in 64 bits.
pub(crate) fn parse_seed(s: Option<&str>) -> Result<Option<u64>> {
    s.map(|s| {
        if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) {
            s.parse().map_err(|_| invalid("seed"))
        } else {
            Err(invalid("seed"))
        }
    })
    .transpose()
}

/// Whether `f` is a schedule fingerprint as stored: [`MIN_FP_CHARS`] to [`MAX_FP_CHARS`] lower case hex digits.
/// Ingest and the `fp` filter both use this rule, so a stored fingerprint is always one a filter can name.
pub(crate) fn is_schedule_fingerprint(f: &str) -> bool {
    (MIN_FP_CHARS..=MAX_FP_CHARS).contains(&f.len())
        && f.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A schedule fingerprint filter. The filter accepts upper case and folds it to the stored lower case.
pub(crate) fn parse_fingerprint(s: Option<&str>) -> Result<Option<String>> {
    match s.map(str::to_ascii_lowercase) {
        None => Ok(None),
        Some(f) if is_schedule_fingerprint(&f) => Ok(Some(f)),
        Some(_) => Err(invalid("fp")),
    }
}

/// How the property table is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropertySort {
    /// Violation, unmet, unrated, pass; then by id.
    Status,
    Id,
    /// Highest point rate first; properties with no rate last.
    Rate,
    /// Most evaluations first.
    Evals,
}

pub(crate) fn parse_sort(s: Option<&str>) -> Result<PropertySort> {
    match s {
        None | Some("status") => Ok(PropertySort::Status),
        Some("id") => Ok(PropertySort::Id),
        Some("rate") => Ok(PropertySort::Rate),
        Some("evals") => Ok(PropertySort::Evals),
        Some(_) => Err(invalid("sort")),
    }
}

/// Filters and paging for a run's property table.
#[derive(Debug, Clone, Default)]
pub struct PropertyQuery {
    pub status: Option<String>,
    pub kind: Option<String>,
    /// Prefix of the property id.
    pub q: Option<String>,
    pub sort: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// Which seeds of one property to list, and where to continue.
#[derive(Debug, Clone, Default)]
pub struct SeedQuery {
    /// `failing` (default), `hit` or `never`.
    pub seeds: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// Where to start, and how many codes, for a page of the seed strip.
#[derive(Debug, Clone, Default)]
pub struct StripQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// Filters and paging for a run's trials.
#[derive(Debug, Clone, Default)]
pub struct TrialQuery {
    pub verdict: Option<String>,
    /// Catalog index: only trials in which that property failed.
    pub fails: Option<String>,
    /// Catalog index: only trials that reached that property.
    pub hits: Option<String>,
    /// Catalog index: only usable trials that did not reach that property.
    pub misses: Option<String>,
    pub degraded: Option<String>,
    pub perturbing: Option<String>,
    /// Schedule fingerprint.
    pub fp: Option<String>,
    /// An exact seed, as a decimal string.
    pub seed: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_are_plain_non_negative_integers() {
        assert_eq!(parse_cursor(None).unwrap(), None);
        assert_eq!(parse_cursor(Some("0")).unwrap(), Some(0));
        assert_eq!(parse_cursor(Some("123")).unwrap(), Some(123));
        for bad in [
            "",
            "-1",
            "1.5",
            "1e3",
            " 1",
            "+1",
            "0x10",
            "99999999999999999999999",
        ] {
            assert!(parse_cursor(Some(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_limit_is_clamped_into_range() {
        assert_eq!(page_size(None, 100, 500), 100);
        assert_eq!(page_size(Some(0), 100, 500), 1);
        assert_eq!(page_size(Some(u32::MAX), 100, 500), 500);
    }

    #[test]
    fn a_fingerprint_is_letters_and_digits_only() {
        assert!(parse_fingerprint(Some("4c16a90b4c16a90b"))
            .unwrap()
            .is_some());
        assert_eq!(
            parse_fingerprint(Some("4C16A90B4C16A90B"))
                .unwrap()
                .as_deref(),
            Some("4c16a90b4c16a90b")
        );
        for bad in [
            "",
            "a b",
            "a'b",
            "a%20b",
            "../x",
            "4c16a90b",
            &"a".repeat(65),
        ] {
            assert!(parse_fingerprint(Some(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn indexes_are_digits_only() {
        assert_eq!(parse_idx(Some("12"), "fails").unwrap(), Some(12));
        for bad in ["", "-1", "1 ", "4294967296", "x"] {
            assert!(parse_idx(Some(bad), "fails").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_error_names_the_parameter_and_never_echoes_the_value() {
        let e = parse_status(Some("<script>")).unwrap_err().to_string();
        assert!(e.contains("status"));
        assert!(!e.contains("script"));
    }
}
