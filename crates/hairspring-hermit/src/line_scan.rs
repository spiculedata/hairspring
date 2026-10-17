//! The lines of a stream that decide a verdict, found while the stream is read, at their true line starts.
//!
//! The capture keeps only the head and the tail of a long stream, and the tail begins at an arbitrary byte. A verdict
//! that searched that text would miss a violation marker in the omitted middle and could take the end of a long line
//! for the start of a marker. So the lines the verdict reads are noted here instead, one line at a time, in memory
//! that does not grow with the stream.

pub const VIOLATION_MARKER: &str = "HAIRSPRING_VIOLATION";
/// Programs written under the project's former name (Anthill) still report violations this way.
pub const LEGACY_VIOLATION_MARKER: &str = "ANTHILL_VIOLATION";

const INTERNAL_FAILURE: &str = "HERMIT_INTERNAL_FAILURE";
const USAGE: &str = "Usage: hermit";
const SKID_OVERSHOOT: &str = "SKID_OVERSHOOT";
const PANICKED: &str = "panicked";
const OVERSHOOT: &str = "overshoot";
/// How the line begins that Hermit prints on stderr when every guest thread is blocked and no timer can wake one, as
/// `Deadlock detected: <what the threads wait on>, but no runnable threads left.`, followed by exit status 1.
const DEADLOCK: &str = "Deadlock detected: ";
const ERROR_UPPER: &str = "Error:";
const ERROR_LOWER: &str = "error:";

/// Most bytes of a line kept to report it. A longer line is cut; a marker's reason this long is already clipped
/// further on.
const LINE_HEAD_BYTES: usize = 1024;

/// What the lines of one stream said. Every field answers a question the verdict asks of the whole stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Findings {
    /// The first line that starts (after leading whitespace) with a violation marker.
    pub violation_line: Option<String>,
    /// Some line contains `HERMIT_INTERNAL_FAILURE`.
    pub internal_failure: bool,
    /// The first line containing `Error:`, trimmed.
    pub error_line: Option<String>,
    /// Some line contains `Usage: hermit`.
    pub usage: bool,
    /// The first line containing `error:`, trimmed.
    pub lower_error_line: Option<String>,
    pub skid_overshoot: bool,
    /// Some line contains `panicked`, and some line (maybe another) contains `overshoot`.
    pub panicked: bool,
    pub overshoot: bool,
    /// The first line containing Hermit's deadlock report, trimmed.
    pub deadlock_line: Option<String>,
}

impl Findings {
    /// The findings for a stream held whole in memory.
    pub fn of_text(text: &str) -> Findings {
        let mut scan = LineScan::default();
        scan.feed(text.as_bytes());
        scan.finish()
    }
}

/// What a line contained, as bits.
const HAS_INTERNAL_FAILURE: u8 = 1;
const HAS_ERROR_UPPER: u8 = 1 << 1;
const HAS_USAGE: u8 = 1 << 2;
const HAS_ERROR_LOWER: u8 = 1 << 3;
const HAS_SKID: u8 = 1 << 4;
const HAS_PANICKED: u8 = 1 << 5;
const HAS_OVERSHOOT: u8 = 1 << 6;

const NEEDLES: [(&str, u8); 7] = [
    (INTERNAL_FAILURE, HAS_INTERNAL_FAILURE),
    (ERROR_UPPER, HAS_ERROR_UPPER),
    (USAGE, HAS_USAGE),
    (ERROR_LOWER, HAS_ERROR_LOWER),
    (SKID_OVERSHOOT, HAS_SKID),
    (PANICKED, HAS_PANICKED),
    (OVERSHOOT, HAS_OVERSHOOT),
];

/// Bytes of the previous piece of a line kept so a needle split across two reads is still found: one less than the
/// longest needle.
const CARRY_BYTES: usize = INTERNAL_FAILURE.len() - 1;

/// Reads a stream in pieces of any size and notes the lines the verdict cares about.
#[derive(Debug, Default)]
pub struct LineScan {
    /// The first bytes of the current line.
    head: Vec<u8>,
    /// The last bytes of the current line so far.
    carry: Vec<u8>,
    seen: u8,
    in_line: bool,
    found: Findings,
}

impl LineScan {
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while !rest.is_empty() {
            let end = rest.iter().position(|b| *b == b'\n');
            self.piece(&rest[..end.unwrap_or(rest.len())]);
            if end.is_some() {
                self.end_line();
            }
            rest = &rest[end.map_or(rest.len(), |i| i + 1)..];
        }
    }

    pub fn finish(mut self) -> Findings {
        if self.in_line {
            self.end_line();
        }
        self.found
    }

    fn piece(&mut self, piece: &[u8]) {
        if piece.is_empty() {
            return;
        }
        self.in_line = true;
        let room = LINE_HEAD_BYTES.saturating_sub(self.head.len());
        self.head.extend_from_slice(&piece[..piece.len().min(room)]);
        let window = [self.carry.as_slice(), piece].concat();
        for (needle, bit) in NEEDLES {
            if contains(&window, needle.as_bytes()) {
                self.seen |= bit;
            }
        }
        let keep_from = window.len().saturating_sub(CARRY_BYTES);
        self.carry = window[keep_from..].to_vec();
    }

    fn end_line(&mut self) {
        let head = std::mem::take(&mut self.head);
        let seen = std::mem::take(&mut self.seen);
        self.carry.clear();
        self.in_line = false;
        let line = String::from_utf8_lossy(&head);
        let line = line.trim_end_matches('\r');
        if self.found.violation_line.is_none() && is_marker_line(line) {
            self.found.violation_line = Some(line.to_string());
        }
        let found = &mut self.found;
        found.internal_failure |= seen & HAS_INTERNAL_FAILURE != 0;
        found.usage |= seen & HAS_USAGE != 0;
        found.skid_overshoot |= seen & HAS_SKID != 0;
        found.panicked |= seen & HAS_PANICKED != 0;
        found.overshoot |= seen & HAS_OVERSHOOT != 0;
        if found.error_line.is_none() && seen & HAS_ERROR_UPPER != 0 {
            found.error_line = Some(line.trim().to_string());
        }
        if found.lower_error_line.is_none() && seen & HAS_ERROR_LOWER != 0 {
            found.lower_error_line = Some(line.trim().to_string());
        }
        // Hermit's report is a whole line that begins `Deadlock detected: `; the same words inside another line are
        // not it.
        if found.deadlock_line.is_none() && line.trim_start().starts_with(DEADLOCK) {
            found.deadlock_line = Some(line.trim().to_string());
        }
    }
}

fn is_marker_line(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with(VIOLATION_MARKER) || l.starts_with(LEGACY_VIOLATION_MARKER)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Findings {
        let mut s = LineScan::default();
        for c in chunks {
            s.feed(c);
        }
        s.finish()
    }

    #[test]
    fn a_marker_at_a_line_start_is_found_with_its_line() {
        let f = scan(&[b"noise\n  HAIRSPRING_VIOLATION: boom\nmore\n"]);
        assert_eq!(
            f.violation_line.as_deref(),
            Some("  HAIRSPRING_VIOLATION: boom")
        );
    }

    #[test]
    fn the_legacy_marker_is_found_too_and_the_first_marker_wins() {
        let f = scan(&[b"ANTHILL_VIOLATION: old\nHAIRSPRING_VIOLATION: new\n"]);
        assert_eq!(f.violation_line.as_deref(), Some("ANTHILL_VIOLATION: old"));
    }

    #[test]
    fn marker_text_inside_a_line_is_not_a_marker() {
        let f = scan(&[b"xxHAIRSPRING_VIOLATION: no\nsee ANTHILL_VIOLATION\n"]);
        assert_eq!(f.violation_line, None);
    }

    #[test]
    fn where_the_reads_end_makes_no_difference() {
        let input = b"a\nHERMIT_INTERNAL_FAILURE\nError: bad\nHAIRSPRING_VIOLATION: x\nUsage: hermit\nerror: y\nSKID_OVERSHOOT\nit panicked\nan overshoot\n";
        let whole = scan(&[input]);
        for i in 0..=input.len() {
            assert_eq!(scan(&[&input[..i], &input[i..]]), whole, "split at {i}");
        }
        assert!(whole.internal_failure && whole.usage && whole.skid_overshoot);
        assert!(whole.panicked && whole.overshoot);
        assert_eq!(whole.error_line.as_deref(), Some("Error: bad"));
        assert_eq!(whole.lower_error_line.as_deref(), Some("error: y"));
    }

    #[test]
    fn a_needle_cannot_be_assembled_from_two_lines() {
        let f = scan(&[b"HERMIT_INTERN\nAL_FAILURE\n"]);
        assert!(!f.internal_failure);
    }

    #[test]
    fn an_unterminated_last_line_counts() {
        let f = scan(&[b"ok\nHAIRSPRING_VIOLATION: last"]);
        assert_eq!(
            f.violation_line.as_deref(),
            Some("HAIRSPRING_VIOLATION: last")
        );
    }

    #[test]
    fn a_very_long_line_is_bounded_but_its_needles_are_still_found() {
        let mut long = vec![b'x'; 5_000_000];
        long.extend_from_slice(b"HERMIT_INTERNAL_FAILURE");
        let f = scan(&[&long]);
        assert!(f.internal_failure);
        assert_eq!(f.violation_line, None);
    }

    #[test]
    fn hermits_deadlock_report_is_found_with_its_line_wherever_the_reads_end() {
        let input = b"hermit: starting\nDeadlock detected: thread(s) waiting on futex, but no runnable threads left.\nlater\n";
        let whole = scan(&[input]);
        assert_eq!(
            whole.deadlock_line.as_deref(),
            Some("Deadlock detected: thread(s) waiting on futex, but no runnable threads left.")
        );
        for i in 0..=input.len() {
            assert_eq!(scan(&[&input[..i], &input[i..]]), whole, "split at {i}");
        }
    }

    #[test]
    fn the_report_must_start_the_line_to_count() {
        let f = scan(&[b"log: Deadlock detected: x\nERROR: Deadlock detected\n"]);
        assert_eq!(f.deadlock_line, None);
        let indented = scan(&[b"  Deadlock detected: x, but no runnable threads left.\r\n"]);
        assert_eq!(
            indented.deadlock_line.as_deref(),
            Some("Deadlock detected: x, but no runnable threads left.")
        );
    }

    #[test]
    fn a_stream_without_the_report_has_no_deadlock_line() {
        assert_eq!(scan(&[b"all fine\n"]).deadlock_line, None);
    }

    #[test]
    fn a_carriage_return_does_not_hide_a_marker_line() {
        let f = scan(&[b"HAIRSPRING_VIOLATION: crlf\r\n"]);
        assert_eq!(
            f.violation_line.as_deref(),
            Some("HAIRSPRING_VIOLATION: crlf")
        );
    }
}
