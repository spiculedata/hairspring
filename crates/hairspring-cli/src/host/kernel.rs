//! The kernel release check. Hermit's ptrace backend uses `PIDFD_THREAD` from Linux 6.9; older kernels take a much
//! slower path for the same work. Determinism is unaffected, throughput is not.

/// The first kernel release with `PIDFD_THREAD`, as (major, minor).
const FAST_PATH_SINCE: (u32, u32) = (6, 9);

/// Reads `major.minor` from a release string such as `6.8.0-146-generic` or `6.9-rc1`. `None` when the string does not
/// start with two numeric components.
pub fn parse_release(release: &str) -> Option<(u32, u32)> {
    let numeric: String = release
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = numeric.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// A warning when the release is known to be older than 6.9. A release that cannot be read gets no warning: a wrong
/// warning is worse than none.
pub fn warning(release: &str) -> Option<String> {
    let version = parse_release(release)?;
    (version < FAST_PATH_SINCE).then(|| {
        format!(
            "kernel {} is older than {}.{}: Hermit's ptrace backend lacks PIDFD_THREAD here and was measured 12 to 38 times \
             slower on the workloads tried. Schedules are unaffected, but trial budgets and timeouts tuned on a newer \
             kernel will be misleading.",
            release.trim(),
            FAST_PATH_SINCE.0,
            FAST_PATH_SINCE.1
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_style_releases_parse_to_major_and_minor() {
        assert_eq!(parse_release("6.8.0-146-generic"), Some((6, 8)));
        assert_eq!(parse_release("6.17.0-42-generic"), Some((6, 17)));
        assert_eq!(parse_release("6.9.0"), Some((6, 9)));
        assert_eq!(parse_release("5.15.0-1-azure\n"), Some((5, 15)));
        assert_eq!(parse_release("6.9-rc1"), Some((6, 9)));
    }

    #[test]
    fn malformed_releases_do_not_parse() {
        for bad in ["", "generic", "6", "6.", ".8", "x.y.z", "6.x"] {
            assert_eq!(parse_release(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_release_before_6_9_warns_and_names_the_release() {
        let text = warning("6.8.0-146-generic").expect("warns");
        assert!(text.contains("6.8.0-146-generic") && text.contains("6.9"));
        assert!(warning("5.15.0-1-azure").is_some());
    }

    #[test]
    fn minor_versions_compare_numerically_not_as_text() {
        assert!(warning("6.9.0").is_none());
        assert!(warning("6.10.1").is_none());
        assert!(warning("6.17.0-42-generic").is_none());
        assert!(warning("7.0.0").is_none());
    }

    #[test]
    fn an_unreadable_release_gets_no_warning() {
        assert!(warning("not a kernel").is_none());
    }
}
