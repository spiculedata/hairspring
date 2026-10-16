//! Splitting one output stream into ordinary output and guest event lines.
//!
//! Ordinary output keeps at most `cap` bytes: the first half and the last half, with the middle read and counted but
//! not stored (see `head_tail`), so a report printed last survives a megabyte of chatter. Lines that start with
//! `HAIRSPRING_EVENT` go to a separate buffer with their own budget, which the plain-output cap never touches, so
//! the guest's protocol lines all reach the parser however much ordinary output surrounds them. The splitter works
//! on the byte stream, one line at a time, and never holds more than one bounded line (plus the two buffers); it
//! does not parse events, that is `hairspring-props`' job.
use crate::head_tail::{HeadTail, Omitted};
use hairspring_props::limits::{EVENT_PREFIX_BARE, MAX_LINE_BYTES};
use std::io::Read;
use std::sync::mpsc;

/// Event lines stored per line: one more than the longest accepted line plus a possible `\r`, enough for the parser
/// to see that a line is too long without keeping the rest of it.
const STORED_LINE_BYTES: usize = MAX_LINE_BYTES + 2;
/// Bytes needed to tell an event line from ordinary output: the bare prefix and the character after it.
const DECIDE_BYTES: usize = EVENT_PREFIX_BARE.len() + 1;
const READ_CHUNK: usize = 8192;

/// Event lines from one stream: whole lines, each ended by `\n`, plus how many were refused for lack of budget.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventLines {
    pub bytes: Vec<u8>,
    pub dropped: u64,
}

impl EventLines {
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty() && self.dropped == 0
    }

    /// The stored lines, without their newlines.
    pub fn lines(&self) -> impl Iterator<Item = &[u8]> {
        self.bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty())
    }
}

/// What one stream produced.
#[derive(Debug, Clone, Default)]
pub struct Captured {
    pub text: String,
    /// What the cap left out of the middle of the ordinary output.
    pub omitted: Omitted,
    pub events: EventLines,
}

enum Line {
    /// Fewer than `DECIDE_BYTES` seen so far.
    Undecided(Vec<u8>),
    Plain,
    /// `stored` is the bounded head of the line, `len` its true length so far.
    Event {
        stored: Vec<u8>,
        len: usize,
    },
}

pub struct Splitter {
    /// Most event-line bytes this stream may store (the trial's budget; the fold applies it to all streams together).
    event_budget: usize,
    kept: HeadTail,
    events: EventLines,
    event_bytes: usize,
    line: Line,
}

/// What follows the bytes examined so far in the current line.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    /// The line ends right here.
    Ends,
    /// More bytes of the line follow in the same read.
    Continues,
    /// Nothing more has arrived yet.
    Unknown,
}

/// Whether a line is an event line, well formed or not, judged from its first bytes. Mirrors the parser: the bare
/// prefix followed by a space, a `{` or a tab (a broken event), or nothing at all; a trailing `\r` is ignored, so
/// `HAIRSPRING_EVENT\r` alone is a broken event while `HAIRSPRING_EVENT\rmore` is ordinary output. `None` means the
/// bytes seen so far cannot tell yet.
fn classify(head: &[u8], next: Next) -> Option<bool> {
    let bare = EVENT_PREFIX_BARE.as_bytes();
    if head.len() < DECIDE_BYTES {
        return (next == Next::Ends).then_some(head == bare);
    }
    if !head.starts_with(bare) {
        return Some(false);
    }
    match head[bare.len()] {
        b' ' | b'{' | b'\t' => Some(true),
        b'\r' => match next {
            Next::Ends => Some(true),
            Next::Continues => Some(false),
            Next::Unknown => None,
        },
        _ => Some(false),
    }
}

impl Splitter {
    #[cfg(test)]
    pub fn new(cap: usize) -> Splitter {
        Splitter::with_event_budget(
            cap,
            hairspring_props::limits::DEFAULT_MAX_EVENT_BYTES_PER_TRIAL,
        )
    }

    pub fn with_event_budget(cap: usize, event_budget: usize) -> Splitter {
        Splitter {
            event_budget,
            kept: HeadTail::new(cap),
            events: EventLines::default(),
            event_bytes: 0,
            line: Line::Undecided(Vec::new()),
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        let mut rest = chunk;
        while !rest.is_empty() {
            let end = rest.iter().position(|b| *b == b'\n');
            let body = &rest[..end.unwrap_or(rest.len())];
            self.absorb(body, end.is_some());
            rest = &rest[end.map_or(rest.len(), |i| i + 1)..];
        }
    }

    pub fn finish(mut self) -> Captured {
        match std::mem::replace(&mut self.line, Line::Plain) {
            Line::Undecided(pending) if classify(&pending, Next::Ends) == Some(true) => {
                self.finish_event(&pending, pending.len())
            }
            Line::Undecided(pending) => self.push_plain(&pending),
            Line::Event { stored, len } => self.finish_event(&stored, len),
            Line::Plain => {}
        }
        let (text, omitted) = self.kept.finish();
        Captured {
            text,
            omitted,
            events: self.events,
        }
    }

    /// `body` is part of the current line; `terminated` says the line ends after it.
    fn absorb(&mut self, body: &[u8], terminated: bool) {
        match &mut self.line {
            Line::Plain => {
                self.push_plain(body);
                if terminated {
                    self.push_plain(b"\n");
                    self.line = Line::Undecided(Vec::new());
                }
            }
            Line::Event { stored, len } => {
                let room = STORED_LINE_BYTES.saturating_sub(stored.len());
                stored.extend_from_slice(&body[..body.len().min(room)]);
                *len += body.len();
                if terminated {
                    let (stored, len) = (std::mem::take(stored), *len);
                    self.finish_event(&stored, len);
                    self.line = Line::Undecided(Vec::new());
                }
            }
            Line::Undecided(pending) => {
                let take = body.len().min(DECIDE_BYTES - pending.len());
                pending.extend_from_slice(&body[..take]);
                let tail = &body[take..];
                let next = match (tail.is_empty(), terminated) {
                    (false, _) => Next::Continues,
                    (true, true) => Next::Ends,
                    (true, false) => Next::Unknown,
                };
                let Some(is_event) = classify(pending, next) else {
                    return;
                };
                let head = std::mem::take(pending);
                self.line = if is_event {
                    let len = head.len();
                    Line::Event { stored: head, len }
                } else {
                    self.push_plain(&head);
                    Line::Plain
                };
                self.absorb(tail, terminated);
            }
        }
    }

    fn push_plain(&mut self, bytes: &[u8]) {
        self.kept.push(bytes);
    }

    /// Stores a finished event line if this stream's budget allows. The cost is computed exactly as the fold computes
    /// it, so within one stream the fold never refuses a line stored here. The fold's budget is per trial, though, and
    /// the file, stdout and stderr share it: up to three streams' worth (3 MiB) may be held here, and whatever the
    /// fold cannot fit, taken in the order file, stdout, stderr, it counts as over the limit.
    fn finish_event(&mut self, stored: &[u8], len: usize) {
        let cost = len.min(MAX_LINE_BYTES) + 1;
        if self.event_bytes.saturating_add(cost) > self.event_budget {
            self.events.dropped += 1;
            return;
        }
        self.event_bytes += cost;
        self.events.bytes.extend_from_slice(stored);
        self.events.bytes.push(b'\n');
    }
}

/// Reads `stream` to its end on a thread and sends what it captured. Read errors end the stream, as before.
pub fn drain(
    mut stream: impl Read + Send + 'static,
    cap: usize,
    event_budget: usize,
    tx: mpsc::Sender<Captured>,
) {
    std::thread::spawn(move || {
        let mut splitter = Splitter::with_event_budget(cap, event_budget);
        let mut buf = [0u8; READ_CHUNK];
        loop {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => splitter.feed(&buf[..n]),
            }
        }
        let _ = tx.send(splitter.finish());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_props::limits::MAX_EVENT_BYTES_PER_TRIAL;

    fn capture(cap: usize, chunks: &[&[u8]]) -> Captured {
        let mut s = Splitter::new(cap);
        for c in chunks {
            s.feed(c);
        }
        s.finish()
    }

    #[test]
    fn plain_output_is_kept_exactly_up_to_the_cap() {
        let c = capture(10, &[b"hello\nworl"]);
        assert_eq!(c.text, "hello\nworl");
        assert!(!c.omitted.is_truncated());
        assert!(c.events.is_empty());
    }

    #[test]
    fn plain_output_over_the_cap_keeps_its_head_and_its_last_line() {
        let c = capture(10, &[b"hello\nworld\nmore\n"]);
        assert_eq!(c.text, "hello\n[... 7 bytes omitted ...]\nmore\n");
        assert_eq!(c.omitted.bytes, 7);
        assert!(c.events.is_empty());
    }

    #[test]
    fn event_lines_do_not_count_against_the_plain_output_cap() {
        let c = capture(
            10,
            &[b"HAIRSPRING_EVENT {\"v\":1}\nhello\nHAIRSPRING_EVENT {\"v\":2}\n"],
        );
        assert_eq!(c.text, "hello\n");
        assert!(!c.omitted.is_truncated());
        assert_eq!(c.events.lines().count(), 2);
    }

    #[test]
    fn event_lines_are_separated_and_leave_no_trace_in_the_text() {
        let c = capture(
            100,
            &[b"a\nHAIRSPRING_EVENT {\"v\":1}\nb\nHAIRSPRING_EVENT {\"v\":2}\n"],
        );
        assert_eq!(c.text, "a\nb\n");
        let lines: Vec<_> = c.events.lines().collect();
        assert_eq!(
            lines,
            vec![
                &b"HAIRSPRING_EVENT {\"v\":1}"[..],
                &b"HAIRSPRING_EVENT {\"v\":2}"[..]
            ]
        );
    }

    #[test]
    fn a_line_split_across_chunks_is_reassembled() {
        let c = capture(100, &[b"HAIRSPR", b"ING_EVENT {\"v\"", b":1}\nok", b"\n"]);
        assert_eq!(c.text, "ok\n");
        assert_eq!(c.events.lines().count(), 1);
    }

    #[test]
    fn the_event_budget_is_a_parameter() {
        let line = b"HAIRSPRING_EVENT {\"v\":1}\n";
        let mut s = Splitter::with_event_budget(100, 2 * (line.len() - 1) + 2);
        for _ in 0..5 {
            s.feed(line);
        }
        let c = s.finish();
        assert_eq!(c.events.lines().count(), 2);
        assert_eq!(c.events.dropped, 3);
    }

    #[test]
    fn an_unterminated_final_event_line_is_kept() {
        let c = capture(100, &[b"x\nHAIRSPRING_EVENT {\"v\":1}"]);
        assert_eq!(c.text, "x\n");
        assert_eq!(c.events.lines().count(), 1);
    }

    #[test]
    fn lookalike_lines_stay_in_the_text() {
        let c = capture(
            200,
            &[b"HAIRSPRING_EVENTS_FILE=/x\nHAIRSPRING_EVEN\nshort\n"],
        );
        assert_eq!(
            c.text,
            "HAIRSPRING_EVENTS_FILE=/x\nHAIRSPRING_EVEN\nshort\n"
        );
        assert!(c.events.is_empty());
    }

    #[test]
    fn a_broken_event_without_separator_counts_as_an_event_line() {
        let c = capture(100, &[b"HAIRSPRING_EVENT{\"v\":1}\nHAIRSPRING_EVENT\n"]);
        assert_eq!(c.text, "");
        assert_eq!(c.events.lines().count(), 2);
    }

    #[test]
    fn a_huge_event_line_is_truncated_but_still_too_long_for_the_parser() {
        let mut line = b"HAIRSPRING_EVENT ".to_vec();
        line.extend(std::iter::repeat_n(b'x', 100_000));
        line.push(b'\n');
        let c = capture(100, &[&line]);
        let stored = c.events.lines().next().unwrap();
        assert!(stored.len() > MAX_LINE_BYTES);
        assert!(stored.len() <= STORED_LINE_BYTES);
    }

    #[test]
    fn event_lines_over_the_budget_are_counted_not_stored() {
        let mut line = b"HAIRSPRING_EVENT ".to_vec();
        line.extend(std::iter::repeat_n(b'x', MAX_LINE_BYTES - line.len()));
        line.push(b'\n');
        let mut s = Splitter::new(10);
        let total = MAX_EVENT_BYTES_PER_TRIAL / line.len() + 10;
        for _ in 0..total {
            s.feed(&line);
        }
        let c = s.finish();
        assert!(c.events.bytes.len() <= MAX_EVENT_BYTES_PER_TRIAL);
        assert!(c.events.dropped >= 10);
        assert_eq!(c.text, "");
    }

    #[test]
    fn a_crlf_event_line_is_an_event_and_a_cr_inside_a_line_is_not() {
        let c = capture(100, &[b"HAIRSPRING_EVENT\r\nok\n"]);
        assert_eq!(c.text, "ok\n");
        assert_eq!(c.events.lines().count(), 1);
        let split = capture(100, &[b"HAIRSPRING_EVENT\r", b"\nok\n"]);
        assert_eq!(split.text, "ok\n");
        assert_eq!(split.events.lines().count(), 1);
        let inside = capture(100, &[b"HAIRSPRING_EVENT\rmore\n"]);
        assert_eq!(inside.text, "HAIRSPRING_EVENT\rmore\n");
        assert!(inside.events.is_empty());
        let at_eof = capture(100, &[b"HAIRSPRING_EVENT\r"]);
        assert_eq!(at_eof.events.lines().count(), 1);
    }

    /// Every way of cutting the stream into two or three reads gives the same result, including a cut exactly where
    /// the splitter decides (after 17 bytes) and one right at a leading newline.
    #[test]
    fn the_result_does_not_depend_on_where_the_reads_end() {
        let input: &[u8] = b"\n\nHAIRSPRING_EVENT {\"v\":1}\nplain\nHAIRSPRING_EVENT\r\nHAIRSPRING_EVENT\nHAIRSPRING_EVENT\rx\nHAIRSPRING_EVEN\nHAIRSPRING_EVENT{\"v\":2}\ntail";
        let whole = capture(1000, &[input]);
        for i in 0..=input.len() {
            let two = capture(1000, &[&input[..i], &input[i..]]);
            assert_eq!(two.text, whole.text, "split at {i}");
            assert_eq!(two.events, whole.events, "split at {i}");
            for j in i..=input.len() {
                let three = capture(1000, &[&input[..i], &input[i..j], &input[j..]]);
                assert_eq!(three.text, whole.text, "split at {i},{j}");
                assert_eq!(three.events, whole.events, "split at {i},{j}");
            }
        }
        assert_eq!(whole.events.lines().count(), 4);
        assert_eq!(
            whole.text,
            "\n\nplain\nHAIRSPRING_EVENT\rx\nHAIRSPRING_EVEN\ntail"
        );
    }

    #[test]
    fn nul_bytes_and_invalid_utf8_in_an_event_line_are_kept_for_the_parser() {
        let c = capture(
            100,
            &[b"HAIRSPRING_EVENT {\"a\":\"\0\xff\xfe\"}\nx\0y\xff\n"],
        );
        assert_eq!(c.events.lines().count(), 1);
        assert!(c.events.bytes.contains(&0));
        assert_eq!(c.text, "x\0y\u{fffd}\n");
    }

    #[test]
    fn a_long_event_line_with_no_newline_at_eof_is_bounded_and_kept() {
        let mut line = b"HAIRSPRING_EVENT ".to_vec();
        line.extend(std::iter::repeat_n(b'x', 1_000_000));
        let c = capture(100, &[&line]);
        let stored = c.events.lines().next().unwrap();
        assert!(stored.len() > MAX_LINE_BYTES && stored.len() <= STORED_LINE_BYTES);
        assert_eq!(c.events.dropped, 0);
    }
}
