//! The guest's own violation marker: a line that begins `HAIRSPRING_VIOLATION`, or the former project name's marker.
//! Every backend reads the same marker the same way.

pub const VIOLATION_MARKER: &str = "HAIRSPRING_VIOLATION";
/// Programs written under the project's former name (Anthill) still report violations this way.
pub const LEGACY_VIOLATION_MARKER: &str = "ANTHILL_VIOLATION";

/// What a violation with no reason text is called.
const NO_REASON: &str = "property violated";

/// Whether `line` is a marker line: the marker, after any leading whitespace, starts it. The same words inside
/// another line are not a marker.
pub fn is_marker_line(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with(VIOLATION_MARKER) || l.starts_with(LEGACY_VIOLATION_MARKER)
}

/// The reason a marker line gives, or `property violated` when it gives none.
pub fn marker_reason(line: &str) -> String {
    let reason = line
        .trim_start()
        .trim_start_matches(VIOLATION_MARKER)
        .trim_start_matches(LEGACY_VIOLATION_MARKER)
        .trim_start_matches(':')
        .trim();
    if reason.is_empty() {
        NO_REASON.to_string()
    } else {
        reason.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_starts_the_line_after_any_whitespace() {
        assert!(is_marker_line("HAIRSPRING_VIOLATION: x"));
        assert!(is_marker_line("   ANTHILL_VIOLATION: x"));
        assert!(!is_marker_line("see HAIRSPRING_VIOLATION: x"));
        assert!(!is_marker_line(""));
    }

    #[test]
    fn the_reason_is_what_follows_the_marker() {
        assert_eq!(marker_reason("HAIRSPRING_VIOLATION: boom"), "boom");
        assert_eq!(marker_reason("  ANTHILL_VIOLATION:   old  "), "old");
        assert_eq!(marker_reason("HAIRSPRING_VIOLATION"), "property violated");
        assert_eq!(marker_reason("HAIRSPRING_VIOLATION: "), "property violated");
    }
}
