//! The per-call-site record that lives in the catalog section, and its compile-time checks.

use std::sync::atomic::Ordering::{Acquire, Relaxed};
use std::sync::atomic::{AtomicI64, AtomicU64};

use crate::fmt::{FILE_MAX, ID_MAX, MSG_MAX};

/// Assertion kinds. The numeric values match the C SDK; 0 marks the sentinel record.
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    None = 0,
    Always = 1,
    AlwaysOrUnreachable = 2,
    Sometimes = 3,
    Reachable = 4,
    Unreachable = 5,
}

impl Kind {
    /// The protocol's name for the kind.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Always => "always",
            Kind::AlwaysOrUnreachable => "always_or_unreachable",
            Kind::Sometimes => "sometimes",
            Kind::Reachable => "reachable",
            Kind::Unreachable => "unreachable",
            Kind::None => "unknown",
        }
    }

    /// `must_hit` as sent in the `catalog` line.
    pub fn must_hit(self) -> bool {
        matches!(self, Kind::Always | Kind::Sometimes | Kind::Reachable)
    }

    /// `reachable` and `unreachable` have no condition: only the true side exists.
    pub fn is_point(self) -> bool {
        matches!(self, Kind::Reachable | Kind::Unreachable)
    }
}

/// Where and by whom an outcome was first seen: the thread slot, that thread's own ordinal, four values.
#[repr(C)]
pub struct Witness {
    pub(crate) slot: AtomicU64,
    pub(crate) ord: AtomicU64,
    pub(crate) vals: [AtomicI64; 4],
}

/// A plain copy of a witness, taken at report time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WitnessSnap {
    pub slot: u64,
    pub ord: u64,
    pub vals: [i64; 4],
}

impl Witness {
    pub const fn new() -> Self {
        Self {
            slot: AtomicU64::new(0),
            ord: AtomicU64::new(0),
            vals: [
                AtomicI64::new(0),
                AtomicI64::new(0),
                AtomicI64::new(0),
                AtomicI64::new(0),
            ],
        }
    }

    /// The witness if one was stored (ordinals start at 1), else `None`.
    pub fn snapshot(&self) -> Option<WitnessSnap> {
        // The writer stores `ord` last, with release, so every other field of that write is visible once it is seen.
        let ord = self.ord.load(Acquire);
        if ord == 0 {
            return None;
        }
        Some(WitnessSnap {
            slot: self.slot.load(Relaxed),
            ord,
            vals: [
                self.vals[0].load(Relaxed),
                self.vals[1].load(Relaxed),
                self.vals[2].load(Relaxed),
                self.vals[3].load(Relaxed),
            ],
        })
    }
}

impl Default for Witness {
    fn default() -> Self {
        Self::new()
    }
}

/// One record per call site, living in the catalog section. The counters are the live state; everything else is
/// fixed at compile time. `align(64)` gives each record its own cache line and makes the records tile the section.
#[repr(C, align(64))]
pub struct Site {
    pub(crate) id: &'static str,
    pub(crate) msg: &'static str,
    pub(crate) file: &'static str,
    pub(crate) line: u32,
    pub(crate) kind: Kind,
    pub(crate) trues: AtomicU64,
    pub(crate) falses: AtomicU64,
    pub(crate) first_true: Witness,
    pub(crate) first_false: Witness,
}

impl Site {
    pub const fn new(
        id: &'static str,
        msg: &'static str,
        file: &'static str,
        line: u32,
        kind: Kind,
    ) -> Self {
        Self {
            id,
            msg,
            file,
            line,
            kind,
            trues: AtomicU64::new(0),
            falses: AtomicU64::new(0),
            first_true: Witness::new(),
            first_false: Witness::new(),
        }
    }

    /// Is this a real call site (not the sentinel that keeps the section in existence)?
    pub fn is_live(&self) -> bool {
        self.kind != Kind::None
    }
}

const fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b':' || c == b'/' || c == b'-'
}

/// Compile-time check of an id: 1 to 128 bytes of `[A-Za-z0-9._:/-]`. Panics (a compile error in a const context).
pub const fn check_id(id: &str) {
    let bytes = id.as_bytes();
    assert!(!bytes.is_empty(), "an id is empty: use 1 to 128 bytes");
    assert!(
        bytes.len() <= ID_MAX,
        "an id is longer than 128 bytes: shorten it"
    );
    let mut i = 0;
    while i < bytes.len() {
        assert!(
            is_ident_byte(bytes[i]),
            "an id may only contain A-Z a-z 0-9 . _ : / -"
        );
        i += 1;
    }
}

/// Compile-time check of a message: at most 512 bytes of printable ASCII without `"` or `\`.
pub const fn check_msg(msg: &str) {
    let bytes = msg.as_bytes();
    assert!(
        bytes.len() <= MSG_MAX,
        "a message is longer than 512 bytes: shorten it"
    );
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        assert!(
            c >= 0x20 && c < 0x7f && c != b'"' && c != b'\\',
            "a message may only contain printable ASCII without a quote or a backslash"
        );
        i += 1;
    }
}

/// The fields of a call site that its catalog line is made from.
#[derive(Clone, Copy)]
pub struct TwinSource {
    pub id: &'static str,
    pub kind_name: &'static str,
    pub must_hit: &'static str,
    pub msg: &'static str,
    pub file: &'static str,
    pub line: u32,
}

/// Where the twin's bytes go. With an empty buffer it only counts, which is how the length is found at compile time.
struct Sink<'a> {
    buf: &'a mut [u8],
    n: usize,
}

impl Sink<'_> {
    const fn byte(&mut self, c: u8) {
        if self.n < self.buf.len() {
            self.buf[self.n] = c;
        }
        self.n += 1;
    }

    const fn raw(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            self.byte(bytes[i]);
            i += 1;
        }
    }

    /// Like `Buf::put_json_text`: at most `max` source bytes from `from`, stopping at a NUL; `"` and `\` escaped;
    /// printable ASCII kept; every other byte becomes `?`.
    const fn json_text(&mut self, s: &[u8], from: usize, max: usize) {
        self.byte(b'"');
        let (mut i, mut taken) = (from, 0);
        while i < s.len() && taken < max && s[i] != 0 {
            let c = s[i];
            if c == b'"' || c == b'\\' {
                self.byte(b'\\');
                self.byte(c);
            } else if c >= 0x20 && c < 0x7f {
                self.byte(c);
            } else {
                self.byte(b'?');
            }
            i += 1;
            taken += 1;
        }
        self.byte(b'"');
    }

    const fn number(&mut self, v: u64) {
        let mut digits = [0u8; 20];
        let (mut i, mut rest) = (digits.len(), v);
        loop {
            i -= 1;
            digits[i] = b'0' + (rest % 10) as u8;
            rest /= 10;
            if rest == 0 {
                break;
            }
        }
        while i < digits.len() {
            self.byte(digits[i]);
            i += 1;
        }
    }
}

/// The twin's text, without the terminating NUL. It is the live `catalog` line minus `proc`.
const fn render(out: &mut Sink, src: &TwinSource) {
    out.raw("{\"v\":1,\"kind\":\"catalog\",\"id\":\"");
    out.raw(src.id);
    out.raw("\",\"type\":\"");
    out.raw(src.kind_name);
    out.raw("\",\"must_hit\":");
    out.raw(src.must_hit);
    out.raw(",\"msg\":");
    out.json_text(src.msg.as_bytes(), 0, MSG_MAX);
    out.raw(",\"loc\":{\"file\":");
    // The path may be longer than the protocol allows; keep its tail, which names the file.
    let file = src.file.as_bytes();
    let from = file.len().saturating_sub(FILE_MAX);
    out.json_text(file, from, FILE_MAX);
    out.raw(",\"line\":");
    out.number(src.line as u64);
    out.raw("}}");
}

/// Length in bytes of the offline twin of a catalog line, its terminating NUL included.
pub const fn twin_len(src: TwinSource) -> usize {
    let mut out = Sink { buf: &mut [], n: 0 };
    render(&mut out, &src);
    out.n + 1
}

/// The offline twin: the JSON of the `catalog` line without `proc`, escaped and trimmed exactly like the live line
/// (the file path keeps its last 256 bytes, `"` and `\` are escaped, other non-printable bytes become `?`), then a NUL.
/// Any bytes of the array past the text stay zero.
pub const fn twin<const N: usize>(src: TwinSource) -> [u8; N] {
    let mut bytes = [0u8; N];
    let mut out = Sink {
        buf: &mut bytes,
        n: 0,
    };
    render(&mut out, &src);
    assert!(out.n < N, "the twin array is too small for the text");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::Relaxed;

    #[test]
    fn a_record_tiles_the_section_without_padding() {
        assert_eq!(std::mem::size_of::<Site>() % 64, 0);
        assert_eq!(std::mem::align_of::<Site>(), 64);
    }

    #[test]
    fn kind_names_and_must_hit_follow_the_protocol() {
        let all = [
            (Kind::Always, "always", true),
            (Kind::AlwaysOrUnreachable, "always_or_unreachable", false),
            (Kind::Sometimes, "sometimes", true),
            (Kind::Reachable, "reachable", true),
            (Kind::Unreachable, "unreachable", false),
        ];
        for (kind, name, must_hit) in all {
            assert_eq!(kind.name(), name);
            assert_eq!(kind.must_hit(), must_hit);
        }
        assert!(Kind::Reachable.is_point() && Kind::Unreachable.is_point());
        assert!(!Kind::Always.is_point() && !Kind::Sometimes.is_point());
    }

    #[test]
    fn a_new_site_starts_with_zero_counters_and_no_witness() {
        let s = Site::new("a.b", "msg", "f.rs", 7, Kind::Always);
        assert_eq!(s.trues.load(Relaxed) + s.falses.load(Relaxed), 0);
        assert!(s.first_true.snapshot().is_none() && s.first_false.snapshot().is_none());
        assert!(s.is_live());
        assert!(!Site::new("", "", "", 0, Kind::None).is_live());
    }

    #[test]
    fn a_stored_witness_reads_back() {
        let w = Witness::new();
        w.slot.store(3, Relaxed);
        w.ord.store(9, Relaxed);
        w.vals[2].store(-5, Relaxed);
        let snap = w.snapshot().unwrap();
        assert_eq!((snap.slot, snap.ord, snap.vals), (3, 9, [0, 0, -5, 0]));
    }

    #[test]
    fn ids_at_the_limits_are_accepted() {
        check_id("a");
        check_id("A-z_0.9:/x");
        check_id(&"x".repeat(ID_MAX));
    }

    #[test]
    #[should_panic(expected = "id is empty")]
    fn an_empty_id_is_refused() {
        check_id("");
    }

    #[test]
    #[should_panic(expected = "id is longer than 128 bytes")]
    fn a_long_id_is_refused() {
        check_id(&"x".repeat(ID_MAX + 1));
    }

    #[test]
    #[should_panic(expected = "id may only contain")]
    fn an_id_with_a_space_is_refused() {
        check_id("a b");
    }

    #[test]
    fn messages_at_the_limit_are_accepted() {
        check_msg("");
        check_msg(&"m".repeat(MSG_MAX));
    }

    #[test]
    #[should_panic(expected = "message is longer than 512 bytes")]
    fn a_long_message_is_refused() {
        check_msg(&"m".repeat(MSG_MAX + 1));
    }

    #[test]
    #[should_panic(expected = "message may only contain printable ASCII")]
    fn a_message_with_a_quote_is_refused() {
        check_msg("say \"hi\"");
    }

    #[test]
    #[should_panic(expected = "message may only contain printable ASCII")]
    fn a_message_with_non_ascii_is_refused() {
        check_msg("\u{2264}");
    }

    /// The live `catalog` line for `src`, without its `proc` member.
    fn live_line(src: &TwinSource) -> String {
        use crate::fmt::Buf;
        use crate::lines::{catalog, CatalogRow, ProcInfo};
        let p = ProcInfo {
            tag: "p1".to_string(),
            tag_stable: true,
            signal_handlers: false,
            catalog_hash: 0,
            n_ids: 0,
            dropped: 0,
        };
        let kind = [
            Kind::Always,
            Kind::AlwaysOrUnreachable,
            Kind::Sometimes,
            Kind::Reachable,
            Kind::Unreachable,
        ]
        .into_iter()
        .find(|k| k.name() == src.kind_name)
        .unwrap();
        let row = CatalogRow {
            id: src.id,
            kind,
            msg: src.msg,
            file: src.file,
            line: src.line,
        };
        let mut b = Buf::new();
        catalog(&mut b, &p, &row).unwrap();
        let text = String::from_utf8(b.as_bytes().to_vec()).unwrap();
        text.trim_end()
            .strip_prefix("HAIRSPRING_EVENT ")
            .unwrap()
            .replace("\"proc\":\"p1\",", "")
    }

    fn twin_text(src: TwinSource) -> String {
        let len = twin_len(src);
        let mut bytes = vec![0u8; len];
        // The const fn is generic over the array length; go through the largest size the test needs.
        let array = twin::<1024>(src);
        bytes.copy_from_slice(&array[..len]);
        assert_eq!(bytes.last(), Some(&0), "NUL terminated");
        String::from_utf8(bytes[..len - 1].to_vec()).unwrap()
    }

    fn source(file: &'static str) -> TwinSource {
        TwinSource {
            id: "a.b",
            kind_name: "sometimes",
            must_hit: "true",
            msg: "say hi",
            file,
            line: 42,
        }
    }

    #[test]
    fn the_twin_equals_the_live_line_for_an_ordinary_path() {
        let src = source("src/lib.rs");
        assert_eq!(twin_text(src), live_line(&src));
    }

    #[test]
    fn the_twin_escapes_a_path_exactly_like_the_live_line() {
        let src = source("dir/we\"ird\\name\u{e9}.rs");
        assert_eq!(twin_text(src), live_line(&src));
        assert!(twin_text(src).contains("we\\\"ird\\\\name??.rs"));
    }

    #[test]
    fn the_twin_keeps_the_last_256_bytes_of_a_long_path() {
        let long: &'static str = Box::leak(format!("{}/tail.rs", "d".repeat(300)).into_boxed_str());
        let src = source(long);
        let text = twin_text(src);
        assert_eq!(text, live_line(&src));
        assert!(text.contains("/tail.rs\""));
        assert!(!text.contains(&"d".repeat(257)));
    }

    #[test]
    fn the_twin_length_counts_every_byte_including_the_nul() {
        let src = source("x.rs");
        assert_eq!(twin_len(src), live_line(&src).len() + 1);
    }

    #[test]
    fn the_record_layout_is_pinned_so_a_change_must_bump_the_section_names() {
        // Two copies of the crate with different layouts must not read each other's records: the section names carry
        // a layout number (see section.rs and macros.rs). Change this number only together with those names.
        assert_eq!(std::mem::size_of::<Site>(), 192);
        assert_eq!(crate::section::LAYOUT, 1);
    }

    #[test]
    fn a_witness_read_while_it_is_written_is_never_torn() {
        // Every field of witness k is k. A reader that sees `ord` set must see all the other fields of that write.
        const N: usize = 200_000;
        let witnesses: Vec<Witness> = (0..N).map(|_| Witness::new()).collect();
        let torn = std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                for (i, w) in witnesses.iter().enumerate() {
                    let k = i as u64 + 1;
                    crate::eval::store(w, k, k, &[k as i64; 4]);
                }
            });
            let mut torn = 0usize;
            let mut seen = 0usize;
            while seen < N {
                seen = 0;
                for (i, w) in witnesses.iter().enumerate() {
                    let k = i as u64 + 1;
                    match w.snapshot() {
                        None => break,
                        Some(s) => {
                            seen += 1;
                            if s.ord != k || s.slot != k || s.vals != [k as i64; 4] {
                                torn += 1;
                            }
                        }
                    }
                }
            }
            writer.join().unwrap();
            torn
        });
        assert_eq!(torn, 0, "witnesses read with a field from another write");
    }
}
