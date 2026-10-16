//! Splitting one output stream into ordinary output and guest event lines.
//!
//! Ordinary output is kept exactly as before: the first `cap` bytes, the rest read and discarded. Lines that start
//! with `HAIRSPRING_EVENT` go to a separate buffer with its own budget, so an end-of-run report printed after a
//! megabyte of chatter is not lost. The splitter works on the byte stream, one line at a time, and never holds more
//! than one bounded line (plus the two buffers); it does not parse events, that is `hairspring-props`' job.
use hairspring_props::limits::{EVENT_PREFIX_BARE, MAX_EVENT_BYTES_PER_TRIAL, MAX_LINE_BYTES};
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
    cap: usize,
    kept: Vec<u8>,
    events: EventLines,
    event_bytes: usize,
    line: Line,
}

/// Whether a line's first bytes mark it as an event line, well formed or not. Mirrors the parser: the bare prefix
/// followed by a space, a `{` or a tab (a broken event), or nothing at all.
fn is_event_start(head: &[u8]) -> bool {
    head.starts_with(EVENT_PREFIX_BARE.as_bytes())
        && matches!(
            head.get(EVENT_PREFIX_BARE.len()),
            None | Some(b' ') | Some(b'{') | Some(b'\t')
        )
}

impl Splitter {
    pub fn new(cap: usize) -> Splitter {
        Splitter {
            cap,
            kept: Vec::new(),
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
            Line::Undecided(pending) if is_event_start(&pending) => {
                self.finish_event(&pending, pending.len())
            }
            Line::Undecided(pending) => self.push_plain(&pending),
            Line::Event { stored, len } => self.finish_event(&stored, len),
            Line::Plain => {}
        }
        Captured {
            text: String::from_utf8_lossy(&self.kept).into_owned(),
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
                if pending.len() < DECIDE_BYTES && !terminated {
                    return;
                }
                let head = std::mem::take(pending);
                self.line = if is_event_start(&head) {
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
        let room = self.cap.saturating_sub(self.kept.len());
        self.kept.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }

    /// Stores a finished event line if the budget allows. The cost matches the parser's own accounting, so a line
    /// stored here is never refused again when the lines are folded.
    fn finish_event(&mut self, stored: &[u8], len: usize) {
        let cost = len.min(MAX_LINE_BYTES) + 1;
        if self.event_bytes.saturating_add(cost) > MAX_EVENT_BYTES_PER_TRIAL {
            self.events.dropped += 1;
            return;
        }
        self.event_bytes += cost;
        self.events.bytes.extend_from_slice(stored);
        self.events.bytes.push(b'\n');
    }
}

/// Reads `stream` to its end on a thread and sends what it captured. Read errors end the stream, as before.
pub fn drain(mut stream: impl Read + Send + 'static, cap: usize, tx: mpsc::Sender<Captured>) {
    std::thread::spawn(move || {
        let mut splitter = Splitter::new(cap);
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

    fn capture(cap: usize, chunks: &[&[u8]]) -> Captured {
        let mut s = Splitter::new(cap);
        for c in chunks {
            s.feed(c);
        }
        s.finish()
    }

    #[test]
    fn plain_output_is_kept_exactly_up_to_the_cap() {
        let c = capture(10, &[b"hello\nworld\nmore\n"]);
        assert_eq!(c.text, "hello\nworl");
        assert!(c.events.is_empty());
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
}
