//! Bounded capture of one stream's ordinary output: the first and the last bytes, and an account of the middle.
//!
//! The head is the first half of the cap, the tail the last half. Memory stays at the cap however long the stream
//! is: the tail is a window that drops its oldest bytes as new ones arrive, and the dropped bytes are counted and
//! hashed so two streams that differ only in what was omitted do not look the same.

/// What a bounded capture left out of a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Omitted {
    /// How many bytes were dropped from the middle of the stream. Zero means the stream was kept whole.
    pub bytes: u64,
    /// FNV-1a over the dropped bytes, in stream order. Meaningful only when `bytes` is not zero.
    pub hash: u64,
}

use std::collections::VecDeque;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

impl Default for Omitted {
    fn default() -> Self {
        Omitted {
            bytes: 0,
            hash: FNV_OFFSET,
        }
    }
}

impl Omitted {
    pub fn is_truncated(&self) -> bool {
        self.bytes > 0
    }

    fn with(self, dropped: &[u8]) -> Omitted {
        Omitted {
            bytes: self.bytes + dropped.len() as u64,
            hash: dropped.iter().fold(self.hash, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
            }),
        }
    }
}

/// The first `ceil(cap / 2)` and the last `floor(cap / 2)` bytes of what is pushed, and an account of the rest.
#[derive(Debug)]
pub struct HeadTail {
    head_cap: usize,
    tail_cap: usize,
    head: Vec<u8>,
    tail: VecDeque<u8>,
    omitted: Omitted,
}

impl HeadTail {
    /// Keeps at most `cap` bytes of a stream, however long it is.
    pub fn new(cap: usize) -> HeadTail {
        HeadTail {
            head_cap: cap - cap / 2,
            tail_cap: cap / 2,
            head: Vec::new(),
            tail: VecDeque::new(),
            omitted: Omitted::default(),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        let into_head = bytes.len().min(self.head_cap - self.head.len());
        self.head.extend_from_slice(&bytes[..into_head]);
        let rest = &bytes[into_head..];
        // Only the last `tail_cap` bytes of what arrives can be kept; the front of the window and the front of
        // `rest` that no longer fit are the omitted middle, in stream order.
        let overflow = (self.tail.len() + rest.len()).saturating_sub(self.tail_cap);
        let from_window = overflow.min(self.tail.len());
        let dropped: Vec<u8> = self.tail.drain(..from_window).collect();
        self.omitted = self.omitted.with(&dropped);
        let from_rest = overflow - from_window;
        self.omitted = self.omitted.with(&rest[..from_rest]);
        self.tail.extend(&rest[from_rest..]);
    }

    /// The kept text, with a marker where bytes were left out, and what was left out.
    pub fn finish(self) -> (String, Omitted) {
        let mut bytes = self.head;
        if self.omitted.is_truncated() {
            bytes.extend_from_slice(omission_marker(self.omitted.bytes).as_bytes());
        }
        bytes.extend(self.tail);
        (String::from_utf8_lossy(&bytes).into_owned(), self.omitted)
    }
}

/// On lines of its own, so a cut in the middle of a line leaves neither half glued to the marker.
fn omission_marker(omitted: u64) -> String {
    format!("\n[... {omitted} bytes omitted ...]\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(cap: usize, chunks: &[&[u8]]) -> (String, Omitted) {
        let mut kept = HeadTail::new(cap);
        for c in chunks {
            kept.push(c);
        }
        kept.finish()
    }

    #[test]
    fn a_stream_within_the_cap_is_kept_whole() {
        let (text, omitted) = capture(10, &[b"hello", b"world"]);
        assert_eq!(text, "helloworld");
        assert_eq!(omitted, Omitted::default());
        assert!(!omitted.is_truncated());
    }

    #[test]
    fn a_longer_stream_keeps_both_ends_and_counts_the_middle() {
        let (text, omitted) = capture(10, &[b"0123456789abcdefghij"]);
        assert_eq!(text, "01234\n[... 10 bytes omitted ...]\nfghij");
        assert_eq!(omitted.bytes, 10);
    }

    #[test]
    fn where_the_chunks_end_makes_no_difference() {
        let input: Vec<u8> = (0..=250u8).cycle().take(5_000).collect();
        let whole = capture(64, &[&input]);
        for size in [1usize, 2, 7, 31, 32, 33, 63, 64, 65, 500] {
            let chunks: Vec<&[u8]> = input.chunks(size).collect();
            assert_eq!(capture(64, &chunks), whole, "chunk size {size}");
        }
        assert_eq!(whole.1.bytes, 5_000 - 64);
    }

    #[test]
    fn memory_stays_at_the_cap_however_long_the_stream() {
        let mut kept = HeadTail::new(100);
        for _ in 0..10_000 {
            kept.push(&[7u8; 1000]);
        }
        assert!(kept.head.len() + kept.tail.len() <= 100);
        assert_eq!(kept.omitted.bytes, 10_000_000 - 100);
    }

    #[test]
    fn two_streams_that_differ_only_in_the_middle_have_different_hashes() {
        let a = capture(4, &[b"ab-----yz"]);
        let b = capture(4, &[b"ab+++++yz"]);
        assert_eq!(a.1.bytes, b.1.bytes);
        assert_ne!(a.1.hash, b.1.hash);
    }

    #[test]
    fn the_last_bytes_are_always_kept() {
        let (text, _) = capture(20, &[&[b'x'; 1000], b"REPORT\n"]);
        assert!(text.ends_with("xxxREPORT\n"), "{text}");
    }

    #[test]
    fn a_cap_of_zero_keeps_nothing_and_counts_everything() {
        let (text, omitted) = capture(0, &[b"abc"]);
        assert_eq!(omitted.bytes, 3);
        assert_eq!(text, "\n[... 3 bytes omitted ...]\n");
    }

    #[test]
    fn an_odd_cap_gives_the_extra_byte_to_the_head() {
        let (text, _) = capture(5, &[b"0123456789"]);
        assert_eq!(text, "012\n[... 5 bytes omitted ...]\n89");
    }
}
