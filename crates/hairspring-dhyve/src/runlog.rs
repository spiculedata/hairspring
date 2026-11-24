//! Reading the VM's `run.log` for the two things the control endpoint needs to know: that a held stop is still held (so a
//! shutdown reaches a stopped guest), and that a RESUME was taken (so a restore is running and not stuck at its stop).
//!
//! A line of interest is `[det_counter=<n>] <text>`. Only complete lines (ending in a newline) are read, because the VM
//! writes one log line in three writes. A restore replays its stop, then logs some lines that belong to the replay itself
//! and are skipped, then logs the line that shows its control connection was made: that is the first sign of RESUME.
//! Two kinds of line are skipped, and a restore whose restored instruction pointer is unknown skips only the second:
//!  1. a debug exit at exactly the restored instruction pointer (`VMEXIT reason=DEBUG(21) rip=0x<hex>`);
//!  2. the one `VTIME 0 -> <vtime> (+<branches> branches)` line whose values are the stop's own.

/// The most of a line shown in a message.
pub const SHOWN_BYTES: usize = 80;
/// The text the VM logs when its control connection is made.
const CONNECT_PREFIX: &str = "director runtime connected";
const EXCLUDED_PREFIXES: [&str; 2] = ["director stale resume", "director unknown command"];

/// The text of the stop line for an event.
pub fn stop_text(stop_seq: u64, branch_count: u64) -> String {
    format!("director stop kind=1 stop_seq={stop_seq} branch_count={branch_count} probe_id=0")
}

/// The text of the connect line for a control socket.
pub fn connect_text(sock: &str) -> String {
    format!("{CONNECT_PREFIX} control={sock}")
}

/// `(counter, text)` of a `[det_counter=<n>] <text>` line.
pub fn parse_det(line: &str) -> Option<(u64, &str)> {
    let rest = line.strip_prefix("[det_counter=")?;
    let (digits, text) = rest.split_once("] ")?;
    if digits.is_empty() || digits.len() > 19 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((digits.parse().ok()?, text))
}

/// The first bytes of a log line as printable ASCII only: a log line can carry the guest's serial text, control
/// characters and escape sequences, so nothing else is ever kept or shown.
pub fn shown(text: &str) -> String {
    text.bytes()
        .take(SHOWN_BYTES)
        .map(|b| {
            if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '?'
            }
        })
        .collect()
}

/// Whether the stop line is the last det line in `raw`: `Some(true)` when no det line follows it (a partial one counts),
/// `Some(false)` when one does, `None` when the stop line is not there.
pub fn stop_held(raw: &[u8], stop_seq: u64, branch_count: u64) -> Option<bool> {
    stop_held_in(raw, stop_seq, branch_count)
}

/// [`stop_held`] on a reader, a line at a time, so a large log is not held in memory.
pub fn stop_held_in<R: std::io::BufRead>(
    mut reader: R,
    stop_seq: u64,
    branch_count: u64,
) -> Option<bool> {
    let stop = stop_text(stop_seq, branch_count);
    let mut seen = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => return seen.then_some(true),
            // A log that cannot be read is not shown to hold the stop: no shutdown is sent on it.
            Err(_) => return None,
            Ok(_) => {}
        }
        let complete = line.last() == Some(&b'\n');
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches('\n');
        if !complete {
            // A partial last line counts as a det line only when it has begun to be one.
            return seen.then(|| !text.starts_with("[det_counter="));
        }
        let Some((_, body)) = parse_det(text) else {
            continue;
        };
        if seen {
            return Some(false);
        }
        seen = body == stop;
    }
}

/// A control stop the VM logged: `director stop kind=<k> stop_seq=<n> branch_count=<n> probe_id=<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopLine {
    pub kind: u64,
}

fn digits(text: &str) -> Option<u64> {
    (!text.is_empty() && text.len() <= 19 && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// The stop line form, with digits only in every number.
pub fn parse_stop(text: &str) -> Option<StopLine> {
    let rest = text.strip_prefix("director stop kind=")?;
    let mut parts = rest.split(' ');
    let kind = digits(parts.next()?)?;
    for key in ["stop_seq=", "branch_count=", "probe_id="] {
        digits(parts.next()?.strip_prefix(key)?)?;
    }
    parts.next().is_none().then_some(StopLine { kind })
}

/// The first control stop logged after the connect line of `sock`. A VM that stops again after it was resumed shows one;
/// which kind it is says what stopped it, and is recorded, not guessed. Reads a line at a time.
pub fn stop_after_connect<R: std::io::BufRead>(reader: R, sock: &str) -> Option<StopLine> {
    let connect = connect_text(sock);
    let mut after = false;
    for line in reader.split(b'\n') {
        let Ok(bytes) = line else {
            return None;
        };
        let text = String::from_utf8_lossy(&bytes);
        let Some((_, body)) = parse_det(&text) else {
            continue;
        };
        if after {
            if let Some(stop) = parse_stop(body) {
                return Some(stop);
            }
        }
        after = after || body == connect;
    }
    None
}

/// What the scan has decided so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanState {
    Pending,
    /// The connect line came first: RESUME was taken.
    Seen,
    /// The first line that counts was the VM refusing the command.
    Excluded(String),
    /// The first line that counts was something else.
    Other(String),
}

/// What a restore's scan needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreScanKeys {
    pub stop_seq: u64,
    pub branch_count: u64,
    pub vtime_ns: u64,
    pub vcpu_id: u32,
    /// The restored instruction pointer; `None` when it could not be read from the checkpoint.
    pub rip: Option<u64>,
    /// The control socket path of this restore.
    pub sock: String,
}

/// An incremental reader of the bytes of `run.log`.
#[derive(Debug, Clone)]
pub struct RestoreScan {
    keys: RestoreScanKeys,
    stop: String,
    connect: String,
    tail: Vec<u8>,
    seen_stop: bool,
    vtime_skipped: bool,
    skipped: u32,
    state: ScanState,
}

impl RestoreScan {
    pub fn new(keys: RestoreScanKeys) -> RestoreScan {
        RestoreScan {
            stop: stop_text(keys.stop_seq, keys.branch_count),
            connect: connect_text(&keys.sock),
            keys,
            tail: Vec::new(),
            seen_stop: false,
            vtime_skipped: false,
            skipped: 0,
            state: ScanState::Pending,
        }
    }

    pub fn state(&self) -> &ScanState {
        &self.state
    }

    /// How many lines of the two skipped kinds have been passed.
    pub fn skipped(&self) -> u32 {
        self.skipped
    }

    fn is_dropped_debug(&self, text: &str) -> bool {
        let Some(rip) = self.keys.rip else {
            return false;
        };
        text.strip_prefix("VMEXIT reason=DEBUG(21) rip=0x")
            .filter(|h| !h.is_empty() && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
            .and_then(|h| u64::from_str_radix(h, 16).ok())
            == Some(rip)
    }

    fn is_start_vtime(&self, text: &str) -> bool {
        if self.vtime_skipped {
            return false;
        }
        let Some(rest) = text.strip_prefix("VTIME 0 -> ") else {
            return false;
        };
        let Some((vtime, tail)) = rest.split_once(" (+") else {
            return false;
        };
        let Some(branches) = tail.strip_suffix(" branches)") else {
            return false;
        };
        digits(vtime) == Some(self.keys.vtime_ns)
            && digits(branches) == Some(self.keys.branch_count)
    }

    /// Takes the next bytes of the log and returns the state.
    pub fn feed(&mut self, data: &[u8]) -> &ScanState {
        if self.state != ScanState::Pending {
            return &self.state;
        }
        self.tail.extend_from_slice(data);
        let buffered = std::mem::take(&mut self.tail);
        let text = String::from_utf8_lossy(&buffered).into_owned();
        let mut parts: Vec<&str> = text.split('\n').collect();
        let tail = parts.pop().unwrap_or("");
        for line in parts {
            let Some((_, body)) = parse_det(line) else {
                continue;
            };
            if !self.seen_stop {
                self.seen_stop = body == self.stop;
                continue;
            }
            if self.is_dropped_debug(body) {
                self.skipped += 1;
                continue;
            }
            if self.is_start_vtime(body) {
                self.vtime_skipped = true;
                self.skipped += 1;
                continue;
            }
            self.state = if body == self.connect {
                ScanState::Seen
            } else if EXCLUDED_PREFIXES.iter().any(|p| body.starts_with(p)) {
                ScanState::Excluded(shown(body))
            } else {
                ScanState::Other(shown(body))
            };
            return &self.state;
        }
        self.tail = tail.as_bytes().to_vec();
        &self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOCK: &str = "/w/state/run/ctl.sock";

    fn keys(rip: Option<u64>) -> RestoreScanKeys {
        RestoreScanKeys {
            stop_seq: 3,
            branch_count: 1000,
            vtime_ns: 5_000,
            vcpu_id: 0,
            rip,
            sock: SOCK.to_string(),
        }
    }

    fn det(n: u64, text: &str) -> String {
        format!("[det_counter={n}] {text}\n")
    }

    fn stop() -> String {
        det(1, &stop_text(3, 1000))
    }

    fn feed_all(scan: &mut RestoreScan, text: &str) -> ScanState {
        scan.feed(text.as_bytes()).clone()
    }

    #[test]
    fn det_lines_parse_strictly() {
        assert_eq!(parse_det("[det_counter=12] hello"), Some((12, "hello")));
        assert_eq!(parse_det("[det_counter=] x"), None);
        assert_eq!(parse_det("[det_counter=1x] x"), None);
        assert_eq!(parse_det("[det_counter=1]x"), None);
        assert_eq!(parse_det("det_counter=1] x"), None);
        assert_eq!(
            parse_det(&format!("[det_counter={}] x", "9".repeat(25))),
            None
        );
    }

    #[test]
    fn the_connect_line_straight_after_the_stop_is_a_resume_that_was_taken() {
        let mut scan = RestoreScan::new(keys(Some(0x1234)));
        let log = format!("{}{}", stop(), det(2, &connect_text(SOCK)));
        assert_eq!(feed_all(&mut scan, &log), ScanState::Seen);
        assert_eq!(scan.skipped(), 0);
    }

    #[test]
    fn a_debug_exit_at_the_restored_rip_and_the_start_vtime_line_are_skipped() {
        let mut scan = RestoreScan::new(keys(Some(0xffff_ffff_8123_4567)));
        let log = [
            stop(),
            det(2, "VMEXIT reason=DEBUG(21) rip=0xffffffff81234567"),
            det(3, "VTIME 0 -> 5000 (+1000 branches)"),
            det(4, &connect_text(SOCK)),
        ]
        .concat();
        assert_eq!(feed_all(&mut scan, &log), ScanState::Seen);
        assert_eq!(scan.skipped(), 2);
    }

    #[test]
    fn a_debug_exit_at_another_rip_decides_the_state_as_other() {
        let mut scan = RestoreScan::new(keys(Some(0x1234)));
        let log = [
            stop(),
            det(2, "VMEXIT reason=DEBUG(21) rip=0x9999"),
            det(3, &connect_text(SOCK)),
        ]
        .concat();
        assert!(
            matches!(feed_all(&mut scan, &log), ScanState::Other(t) if t.contains("rip=0x9999"))
        );
    }

    #[test]
    fn with_the_rip_unknown_no_debug_line_is_skipped() {
        let mut scan = RestoreScan::new(keys(None));
        let log = [
            stop(),
            det(2, "VMEXIT reason=DEBUG(21) rip=0x1234"),
            det(3, &connect_text(SOCK)),
        ]
        .concat();
        assert!(matches!(feed_all(&mut scan, &log), ScanState::Other(_)));
    }

    #[test]
    fn only_the_first_start_vtime_line_with_the_stops_own_values_is_skipped() {
        let mut scan = RestoreScan::new(keys(None));
        let log = [
            stop(),
            det(2, "VTIME 0 -> 5000 (+1000 branches)"),
            det(3, "VTIME 0 -> 5000 (+1000 branches)"),
        ]
        .concat();
        assert!(matches!(feed_all(&mut scan, &log), ScanState::Other(_)));
        let mut scan = RestoreScan::new(keys(None));
        let log = [stop(), det(2, "VTIME 0 -> 5001 (+1000 branches)")].concat();
        assert!(matches!(feed_all(&mut scan, &log), ScanState::Other(_)));
    }

    #[test]
    fn a_refused_command_is_excluded() {
        for text in [
            "director stale resume stop_seq=2",
            "director unknown command 9",
        ] {
            let mut scan = RestoreScan::new(keys(None));
            let log = [stop(), det(2, text)].concat();
            assert!(
                matches!(feed_all(&mut scan, &log), ScanState::Excluded(t) if t.starts_with("director")),
                "{text}"
            );
        }
    }

    #[test]
    fn the_connect_line_of_another_socket_is_not_this_restores() {
        let mut scan = RestoreScan::new(keys(None));
        let log = [stop(), det(2, &connect_text("/other/ctl.sock"))].concat();
        assert!(matches!(feed_all(&mut scan, &log), ScanState::Other(_)));
    }

    #[test]
    fn lines_before_the_stop_line_and_lines_that_are_not_det_lines_are_ignored() {
        let mut scan = RestoreScan::new(keys(None));
        let log = [
            "plain stderr line\n".to_string(),
            det(1, "something before"),
            stop().replace("[det_counter=1]", "[det_counter=2]"),
            "another plain line\n".to_string(),
            det(3, &connect_text(SOCK)),
        ]
        .concat();
        assert_eq!(feed_all(&mut scan, &log), ScanState::Seen);
    }

    #[test]
    fn a_partial_line_waits_for_its_newline_across_feeds() {
        let mut scan = RestoreScan::new(keys(None));
        let whole = [stop(), det(2, &connect_text(SOCK))].concat();
        let (first, second) = whole.split_at(whole.len() - 10);
        assert_eq!(feed_all(&mut scan, first), ScanState::Pending);
        assert_eq!(feed_all(&mut scan, second), ScanState::Seen);
    }

    #[test]
    fn once_decided_the_state_does_not_change() {
        let mut scan = RestoreScan::new(keys(None));
        let log = [
            stop(),
            det(2, &connect_text(SOCK)),
            det(3, "director stale resume"),
        ]
        .concat();
        assert_eq!(feed_all(&mut scan, &log), ScanState::Seen);
        assert_eq!(feed_all(&mut scan, "[det_counter=9] x\n"), ScanState::Seen);
    }

    #[test]
    fn a_shown_line_is_cut_to_printable_ascii_whatever_it_held() {
        let mut scan = RestoreScan::new(keys(None));
        let long = "\u{e9}".repeat(100);
        let log = [stop(), det(2, &long)].concat();
        let ScanState::Other(t) = feed_all(&mut scan, &log) else {
            panic!("not other");
        };
        assert!(
            t.len() <= SHOWN_BYTES && t.bytes().all(|b| (0x20..0x7f).contains(&b)),
            "{t:?}"
        );
    }

    #[test]
    fn a_held_stop_has_no_det_line_after_it() {
        let held = stop();
        assert_eq!(stop_held(held.as_bytes(), 3, 1000), Some(true));
        let moved = [stop(), det(2, "VMEXIT x")].concat();
        assert_eq!(stop_held(moved.as_bytes(), 3, 1000), Some(false));
        let partial = [stop(), "[det_counter=2] VM".to_string()].concat();
        assert_eq!(stop_held(partial.as_bytes(), 3, 1000), Some(false));
        let plain_tail = [stop(), "stderr noise".to_string()].concat();
        assert_eq!(stop_held(plain_tail.as_bytes(), 3, 1000), Some(true));
        assert_eq!(stop_held(b"", 3, 1000), None);
        assert_eq!(stop_held(det(1, "other").as_bytes(), 3, 1000), None);
    }

    #[test]
    fn a_stop_after_the_connect_line_is_found_with_its_kind_and_one_before_it_is_not() {
        let after = [
            stop(),
            det(2, &connect_text(SOCK)),
            det(
                3,
                "director stop kind=2 stop_seq=9 branch_count=4000000000 probe_id=0",
            ),
        ]
        .concat();
        assert_eq!(
            stop_after_connect(after.as_bytes(), SOCK),
            Some(StopLine { kind: 2 })
        );
        let other_kind = [
            det(1, &connect_text(SOCK)),
            det(
                2,
                "director stop kind=1 stop_seq=9 branch_count=5 probe_id=0",
            ),
        ]
        .concat();
        assert_eq!(
            stop_after_connect(other_kind.as_bytes(), SOCK),
            Some(StopLine { kind: 1 })
        );
        let before = [stop(), det(2, &connect_text(SOCK)), det(3, "VMEXIT x")].concat();
        assert_eq!(stop_after_connect(before.as_bytes(), SOCK), None);
        assert_eq!(stop_after_connect(stop().as_bytes(), SOCK), None);
        let other_sock = [
            det(1, &connect_text("/other")),
            det(
                2,
                "director stop kind=2 stop_seq=1 branch_count=1 probe_id=0",
            ),
        ]
        .concat();
        assert_eq!(stop_after_connect(other_sock.as_bytes(), SOCK), None);
        assert_eq!(stop_after_connect(&b"\xff\xfe"[..], SOCK), None);
    }

    #[test]
    fn only_the_exact_stop_line_form_with_plain_digits_counts() {
        assert_eq!(
            parse_stop("director stop kind=1 stop_seq=2 branch_count=3 probe_id=4"),
            Some(StopLine { kind: 1 })
        );
        for bad in [
            "director stop kind=+1 stop_seq=2 branch_count=3 probe_id=4",
            "director stop kind=1 stop_seq=2 branch_count=3",
            "director stop kind=1 stop_seq=2 branch_count=3 probe_id=4 extra",
            "director stop kind=x stop_seq=2 branch_count=3 probe_id=4",
            "director stopped kind=1 stop_seq=2 branch_count=3 probe_id=4",
            "director stop kind=1 stop_seq=2 probe_id=4 branch_count=3",
        ] {
            assert_eq!(parse_stop(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_signed_number_in_the_start_time_line_is_not_the_stops_own() {
        let mut scan = RestoreScan::new(keys(None));
        let log = [stop(), det(2, "VTIME 0 -> +5000 (+1000 branches)")].concat();
        assert!(matches!(feed_all(&mut scan, &log), ScanState::Other(_)));
    }

    #[test]
    fn shown_text_is_printable_ascii_and_short() {
        let nasty = "deterministic \u{1b}[31m red \u{e9} \u{0}";
        let out = shown(nasty);
        assert!(out.bytes().all(|b| (0x20..0x7f).contains(&b)), "{out:?}");
        assert!(!out.contains('\u{1b}'));
        assert!(shown(&"a".repeat(500)).len() <= SHOWN_BYTES);
    }

    /// A reader that gives its bytes and then an error.
    struct Failing<'a>(&'a [u8], bool);

    impl std::io::Read for Failing<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0.is_empty() {
                self.1 = true;
                return Err(std::io::Error::other("disk"));
            }
            let n = self.0.len().min(buf.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    #[test]
    fn a_read_error_after_the_stop_line_is_not_a_held_stop() {
        let log = stop();
        let reader = std::io::BufReader::new(Failing(log.as_bytes(), false));
        assert_eq!(stop_held_in(reader, 3, 1000), None);
        let reader = std::io::BufReader::new(Failing(b"", false));
        assert_eq!(stop_held_in(reader, 3, 1000), None);
    }

    #[test]
    fn the_held_check_reads_a_reader_too() {
        let held = stop();
        assert_eq!(stop_held_in(held.as_bytes(), 3, 1000), Some(true));
        let moved = [stop(), det(2, "x")].concat();
        assert_eq!(stop_held_in(moved.as_bytes(), 3, 1000), Some(false));
        assert_eq!(stop_held_in(&b""[..], 3, 1000), None);
    }
}
