//! Reads a stream a piece at a time and notes the lines that decide a verdict, at their true line starts.
//!
//! A backend keeps only the head and the tail of a long stream, and the tail begins at an arbitrary byte. A verdict
//! that searched that text would miss a violation marker in the omitted middle and could take the end of a long line
//! for the start of a marker. So the lines the verdict reads are noted here instead, one line at a time, in memory
//! that does not grow with the stream.
//!
//! The scan finds the guest's violation marker itself. A backend adds its own needles (words its tooling prints) and
//! decides what a line holding them means, through [`LineRules`].
use super::markers::is_marker_line;

/// Most bytes of a line kept to report it. A longer line is cut; a marker's reason this long is already clipped
/// further on.
const LINE_HEAD_BYTES: usize = 1024;

/// What a backend notes about each line, beyond the violation marker.
pub trait LineRules: Default {
    /// Substrings looked for anywhere in a line, even when a read ends in the middle of one. At most 32.
    const NEEDLES: &'static [&'static str];

    /// Called once per finished line (carriage return removed, cut to its first bytes) with a bit set for each
    /// needle the whole line contained: bit `i` is `NEEDLES[i]`.
    fn line(&mut self, line: &str, seen: u32);
}

/// What a finished scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scanned<R> {
    /// The first line that starts (after leading whitespace) with a violation marker.
    pub violation_line: Option<String>,
    pub rules: R,
}

/// Reads a stream in pieces of any size.
#[derive(Debug, Default)]
pub struct LineScan<R: LineRules> {
    /// The first bytes of the current line.
    head: Vec<u8>,
    /// The last bytes of the current line so far: one less than the longest needle, so a needle split across two
    /// reads is still found.
    carry: Vec<u8>,
    seen: u32,
    in_line: bool,
    violation_line: Option<String>,
    rules: R,
}

impl<R: LineRules> LineScan<R> {
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

    pub fn finish(mut self) -> Scanned<R> {
        if self.in_line {
            self.end_line();
        }
        Scanned {
            violation_line: self.violation_line,
            rules: self.rules,
        }
    }

    fn piece(&mut self, piece: &[u8]) {
        if piece.is_empty() {
            return;
        }
        self.in_line = true;
        let room = LINE_HEAD_BYTES.saturating_sub(self.head.len());
        self.head.extend_from_slice(&piece[..piece.len().min(room)]);
        let window = [self.carry.as_slice(), piece].concat();
        for (bit, needle) in R::NEEDLES.iter().enumerate() {
            if contains(&window, needle.as_bytes()) {
                self.seen |= 1 << bit;
            }
        }
        let keep_from = window.len().saturating_sub(carry_bytes::<R>());
        self.carry = window[keep_from..].to_vec();
    }

    fn end_line(&mut self) {
        let head = std::mem::take(&mut self.head);
        let seen = std::mem::take(&mut self.seen);
        self.carry.clear();
        self.in_line = false;
        let line = String::from_utf8_lossy(&head);
        let line = line.trim_end_matches('\r');
        if self.violation_line.is_none() && is_marker_line(line) {
            self.violation_line = Some(line.to_string());
        }
        self.rules.line(line, seen);
    }
}

fn carry_bytes<R: LineRules>() -> usize {
    R::NEEDLES
        .iter()
        .map(|n| n.len())
        .max()
        .unwrap_or(0)
        .saturating_sub(1)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Notes which lines held which needles, and nothing else.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Words(Vec<(String, u32)>);

    impl LineRules for Words {
        const NEEDLES: &'static [&'static str] = &["alpha", "beta-long-needle"];

        fn line(&mut self, line: &str, seen: u32) {
            if seen != 0 {
                self.0.push((line.to_string(), seen));
            }
        }
    }

    fn scan(chunks: &[&[u8]]) -> Scanned<Words> {
        let mut s = LineScan::<Words>::default();
        for c in chunks {
            s.feed(c);
        }
        s.finish()
    }

    #[test]
    fn a_marker_at_a_line_start_is_found_with_its_line_and_the_first_wins() {
        let f = scan(&[b"noise\n  HAIRSPRING_VIOLATION: boom\nANTHILL_VIOLATION: later\n"]);
        assert_eq!(
            f.violation_line.as_deref(),
            Some("  HAIRSPRING_VIOLATION: boom")
        );
    }

    #[test]
    fn marker_text_inside_a_line_is_not_a_marker() {
        assert_eq!(
            scan(&[b"xxHAIRSPRING_VIOLATION: no\n"]).violation_line,
            None
        );
    }

    #[test]
    fn needles_are_reported_per_line_with_their_bits() {
        let f = scan(&[b"one alpha\nnothing\nbeta-long-needle and alpha\r\n"]);
        assert_eq!(
            f.rules.0,
            vec![
                ("one alpha".to_string(), 0b01),
                ("beta-long-needle and alpha".to_string(), 0b11)
            ]
        );
    }

    #[test]
    fn where_the_reads_end_makes_no_difference() {
        let input = b"a\nbeta-long-needle\nHAIRSPRING_VIOLATION: x\nalpha\nlast alpha";
        let whole = scan(&[input]);
        for i in 0..=input.len() {
            assert_eq!(scan(&[&input[..i], &input[i..]]), whole, "split at {i}");
        }
        assert_eq!(whole.rules.0.len(), 3);
    }

    #[test]
    fn a_needle_cannot_be_assembled_from_two_lines() {
        assert!(scan(&[b"alp\nha\n"]).rules.0.is_empty());
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
        long.extend_from_slice(b"alpha");
        let f = scan(&[&long]);
        assert_eq!(f.rules.0.len(), 1);
        assert!(f.rules.0[0].0.len() <= LINE_HEAD_BYTES);
        assert_eq!(f.violation_line, None);
    }
}
