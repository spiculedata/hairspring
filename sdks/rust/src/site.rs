//! The per-call-site record that lives in the catalog section, and its compile-time checks.

use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicI64, AtomicU64};

use crate::fmt::{ID_MAX, MSG_MAX};

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
        let ord = self.ord.load(Relaxed);
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

/// Copy a string's bytes into an array, for the offline catalog twin placed in its own section.
pub const fn twin_bytes<const N: usize>(s: &str) -> [u8; N] {
    let bytes = s.as_bytes();
    assert!(bytes.len() == N, "the twin array must match the text");
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        out[i] = bytes[i];
        i += 1;
    }
    out
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

    #[test]
    fn twin_bytes_copies_the_text() {
        const TEXT: &str = "abc\0";
        const BYTES: [u8; TEXT.len()] = twin_bytes::<{ TEXT.len() }>(TEXT);
        assert_eq!(&BYTES, b"abc\0");
    }
}
