//! Generated-input tests for the code that reads a guest's raw output streams: the splitter, the head and tail
//! capture and the line scanner. Streams are built from awkward pieces (very long lines, invalid UTF-8, carriage
//! returns, lines that never end, event prefixes cut in half) and fed in random chunk sizes. The reads of a pipe can
//! end anywhere, so where they end must never change what was captured, and what is kept must stay inside its caps.
//!
//! The default case count is small enough for CI. `HAIRSPRING_FUZZ_CASES` raises it (see docs/testing.md).

use crate::capture::{Captured, Splitter};
use crate::events::fold_sources;
use crate::line_scan::{Findings, LineScan};
use hairspring_props::limits::*;
use hairspring_props::TrialEvents;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

fn cases(default: usize) -> usize {
    std::env::var("HAIRSPRING_FUZZ_CASES")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn base_seed() -> u64 {
    std::env::var("HAIRSPRING_FUZZ_SEED")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Runs one generated case. A failing assertion inside `body` is re-raised with the case number, the seed the case was
/// built from and the base seed, so any failure can be repeated from the message alone.
fn check_case(case: usize, seed: u64, body: impl FnOnce()) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    if let Err(payload) = result {
        let reason = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_else(|| "a panic with no text".to_string());
        panic!(
            "case {case}, seed {seed} (HAIRSPRING_FUZZ_SEED={}): {reason}",
            base_seed()
        );
    }
}

const PIECES: &[&[u8]] = &[
    b"plain output\n",
    b"HAIRSPRING_EVENT {\"v\":1,\"kind\":\"work\",\"proc\":\"p\",\"ops\":3}\n",
    b"HAIRSPRING_EVENT {\"v\":1,\"kind\":\"catalog\",\"proc\":\"p\",\"id\":\"a\",\"type\":\"always\",\"must_hit\":true}\n",
    b"HAIRSPRING_EVENT {\"v\":1,\"kind\":\"assert\",\"proc\":\"p\",\"id\":\"a\",\"hits\":2,\"trues\":1,\"falses\":1,\"first_false\":null,\"first_true\":null}\n",
    b"HAIRSPRING_EVENT\n",
    b"HAIRSPRING_EVENT\r\n",
    b"HAIRSPRING_EVENT\rmore\n",
    b"HAIRSPRING_EVENT{}\n",
    b"HAIRSPRING_EVENT\t{}\n",
    b"HAIRSPRING_EVENTS_FILE=/tmp/x\n",
    b"HAIRSPRING_EVEN",
    b"HAIRSPRING_EVENT ",
    b"HAIRSPRING_VIOLATION: boom\n",
    b"ANTHILL_VIOLATION: old\r\n",
    b"  Deadlock detected: x, but no runnable threads left.\n",
    b"HERMIT_INTERNAL_FAILURE\n",
    b"it panicked at overshoot\n",
    b"Error: bad\nerror: worse\nUsage: hermit\nSKID_OVERSHOOT\n",
    b"\xff\xfe\xc0\xaf\n",
    b"\xed\xa0\x80\n",
    b"\r\r\r\n",
    b"\n\n\n",
    b"\0\0\0\n",
    b"\xe2\x80\xae bidi \xe2\x80\xac\n",
];

fn stream(rng: &mut Rng) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..rng.below(40) {
        match rng.below(12) {
            0 => {
                // A long line: an event line or plain output past the line limit, some of it past the cap.
                let len = [
                    MAX_LINE_BYTES - 1,
                    MAX_LINE_BYTES,
                    MAX_LINE_BYTES + 1,
                    9_000,
                    70_000,
                ][rng.below(5)];
                if rng.chance(2) {
                    out.extend_from_slice(b"HAIRSPRING_EVENT ");
                }
                out.extend(std::iter::repeat_n(b'x', len));
                if !rng.chance(4) {
                    out.push(b'\n');
                }
            }
            1 => out.extend((0..rng.below(100)).map(|_| rng.next() as u8)),
            _ => out.extend_from_slice(PIECES[rng.below(PIECES.len())]),
        }
    }
    if rng.chance(3) && !out.is_empty() {
        out.truncate(rng.below(out.len()));
    }
    out
}

/// Cuts `bytes` into chunks of random sizes, including empty ones and single bytes.
fn chunks<'a>(rng: &mut Rng, bytes: &'a [u8]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let size = match rng.below(4) {
            0 => 0,
            1 => 1,
            2 => 1 + rng.below(20),
            _ => 1 + rng.below(9000),
        };
        let (head, tail) = rest.split_at(size.min(rest.len()));
        out.push(head);
        rest = tail;
    }
    out
}

fn capture(cap: usize, budget: usize, pieces: &[&[u8]]) -> Captured {
    let mut splitter = Splitter::with_event_budget(cap, budget);
    for piece in pieces {
        splitter.feed(piece);
    }
    splitter.finish()
}

fn same(a: &Captured, b: &Captured) -> bool {
    a.text == b.text && a.omitted == b.omitted && a.findings == b.findings && a.events == b.events
}

#[test]
fn where_the_reads_end_never_changes_what_the_splitter_captures() {
    for case in 0..cases(200) {
        let seed = base_seed() + case as u64 + 1;
        check_case(case, seed, || {
            let mut rng = Rng::new(seed);
            let bytes = stream(&mut rng);
            let cap = [0usize, 1, 7, 64, 4096][rng.below(5)];
            let budget = [0usize, 100, 5_000, DEFAULT_MAX_EVENT_BYTES_PER_TRIAL][rng.below(4)];
            let whole = capture(cap, budget, &[&bytes]);
            let split = capture(cap, budget, &chunks(&mut rng, &bytes));
            assert!(
                same(&whole, &split),
                "case {case}: chunking changed the capture"
            );

            // Caps: the kept text is the cap plus a marker, widened at most threefold by lossy decoding.
            assert!(
                whole.text.len() <= cap * 3 + 64,
                "case {case}: text {} for cap {cap}",
                whole.text.len()
            );
            // Each stored event line costs at most its bounded length plus the newline, and the budget is checked on
            // the bounded length, so the store can pass the budget only by the two bytes a stored line keeps beyond it.
            let lines = whole.events.bytes.iter().filter(|b| **b == b'\n').count();
            assert!(
                whole.events.bytes.len() <= budget + 2 * lines,
                "case {case}"
            );
            for line in whole.events.lines() {
                assert!(
                    line.len() <= MAX_LINE_BYTES + 2,
                    "case {case}: stored line of {}",
                    line.len()
                );
            }
            let findings_bound = 3 * 1024 + 64;
            for text in [
                &whole.findings.violation_line,
                &whole.findings.error_line,
                &whole.findings.lower_error_line,
                &whole.findings.deadlock_line,
            ]
            .into_iter()
            .flatten()
            {
                assert!(
                    text.len() <= findings_bound,
                    "case {case}: finding of {} bytes",
                    text.len()
                );
            }
        });
    }
}

#[test]
fn the_events_the_splitter_keeps_fold_to_what_the_whole_stream_folds_to() {
    for case in 0..cases(200) {
        let seed = base_seed() + 0x5000 + case as u64;
        check_case(case, seed, || {
            let mut rng = Rng::new(seed);
            let bytes = stream(&mut rng);
            let budget = DEFAULT_MAX_EVENT_BYTES_PER_TRIAL;
            let captured = capture(4096, budget, &chunks(&mut rng, &bytes));
            let direct = TrialEvents::from_output(&bytes).fold();
            let empty = crate::capture::EventLines::default();
            match fold_sources(None, &captured.events, &empty, budget) {
                Some(via_splitter) => assert_eq!(via_splitter, direct, "case {case}"),
                // Nothing that looked like an event reached the fold, so the whole-stream fold must have found none.
                None => assert_eq!(direct.health.event_bytes, 0, "case {case}"),
            }
        });
    }
}

#[test]
fn the_line_scanner_agrees_with_itself_wherever_the_reads_end() {
    for case in 0..cases(400) {
        let seed = base_seed() + 0x9000 + case as u64;
        check_case(case, seed, || {
            let mut rng = Rng::new(seed);
            let bytes = stream(&mut rng);
            let whole: Findings = {
                let mut scan = LineScan::default();
                scan.feed(&bytes);
                scan.finish()
            };
            let mut scan = LineScan::default();
            for piece in chunks(&mut rng, &bytes) {
                scan.feed(piece);
            }
            assert_eq!(scan.finish(), whole, "case {case}");
        });
    }
}
