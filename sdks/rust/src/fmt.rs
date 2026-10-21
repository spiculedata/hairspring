//! Allocation-free line formatting. The widths and padding match the C SDK so a line's length never depends on a
//! counter's value (see docs/rust-sdk.md).

pub const LINE_MAX: usize = 4096;
pub const NUM_WIDTH: usize = 20;
pub const ID_MAX: usize = 128;
pub const THREAD_KEY_MAX: usize = 64;
pub const MSG_MAX: usize = 512;
pub const FILE_MAX: usize = 256;
pub const TAG_MAX: usize = 64;

/// A bounded output buffer on the stack. Once `over` is set every later put is ignored, so the caller checks once.
pub struct Buf {
    bytes: [u8; LINE_MAX],
    len: usize,
    over: bool,
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

impl Buf {
    pub fn new() -> Self {
        Self {
            bytes: [0; LINE_MAX],
            len: 0,
            over: false,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub fn is_over(&self) -> bool {
        self.over
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.over = false;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    fn put_byte(&mut self, c: u8) {
        if self.over {
            return;
        }
        if self.len >= LINE_MAX {
            self.over = true;
            return;
        }
        self.bytes[self.len] = c;
        self.len += 1;
    }

    pub fn put_bytes(&mut self, s: &[u8]) {
        for &c in s {
            self.put_byte(c);
        }
    }

    pub fn put_str(&mut self, s: &str) {
        self.put_bytes(s.as_bytes());
    }

    /// `n` spaces.
    pub fn pad(&mut self, n: usize) {
        for _ in 0..n {
            self.put_byte(b' ');
        }
    }

    pub fn put_u64(&mut self, v: u64) {
        let (digits, first) = digits_of(v);
        self.put_bytes(&digits[first..]);
    }

    /// Right-aligned in `NUM_WIDTH` characters.
    pub fn put_u64_w(&mut self, v: u64) {
        let (digits, first) = digits_of(v);
        self.pad(first);
        self.put_bytes(&digits[first..]);
    }

    pub fn put_i64(&mut self, v: i64) {
        if v < 0 {
            self.put_byte(b'-');
        }
        self.put_u64(v.unsigned_abs());
    }

    /// Right-aligned in `NUM_WIDTH` characters, the sign inside the field.
    pub fn put_i64_w(&mut self, v: i64) {
        let (digits, first) = digits_of(v.unsigned_abs());
        let used = NUM_WIDTH - first + usize::from(v < 0);
        self.pad(NUM_WIDTH - used);
        if v < 0 {
            self.put_byte(b'-');
        }
        self.put_bytes(&digits[first..]);
    }

    /// Sixteen lowercase hex digits.
    pub fn put_hex64(&mut self, v: u64) {
        for shift in (0..16).rev() {
            let nibble = ((v >> (shift * 4)) & 0xf) as u8;
            self.put_byte(if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            });
        }
    }

    /// A quoted JSON string of at most `max` source bytes. Printable ASCII survives; `"` and `\` are escaped; every
    /// other byte becomes `?`, which keeps free text inside the protocol's string rules without UTF-8 decoding.
    pub fn put_json_text(&mut self, s: &[u8], max: usize) {
        self.put_byte(b'"');
        for &c in s.iter().take(max).take_while(|&&c| c != 0) {
            match c {
                b'"' | b'\\' => {
                    self.put_byte(b'\\');
                    self.put_byte(c);
                }
                0x20..=0x7e => self.put_byte(c),
                _ => self.put_byte(b'?'),
            }
        }
        self.put_byte(b'"');
    }

    /// A quoted id or process tag: bytes outside `[A-Za-z0-9._:/-]` become `_`.
    pub fn put_ident(&mut self, s: &[u8], max: usize) {
        self.put_byte(b'"');
        for &c in s.iter().take(max).take_while(|&&c| c != 0) {
            self.put_byte(sent_char(c));
        }
        self.put_byte(b'"');
    }

    /// A quoted thread key: like an id, but `#` is allowed (claim-order keys are `t#<n>`).
    pub fn put_thread_key(&mut self, s: &[u8], max: usize) {
        self.put_byte(b'"');
        for &c in s.iter().take(max).take_while(|&&c| c != 0) {
            self.put_byte(if c == b'#' { c } else { sent_char(c) });
        }
        self.put_byte(b'"');
    }

    /// The same, followed by spaces up to `THREAD_KEY_MAX` key bytes, so a line's length does not depend on the key.
    pub fn put_thread_key_padded(&mut self, s: &[u8]) {
        let start = self.len;
        self.put_thread_key(s, THREAD_KEY_MAX);
        let used = self.len - start - 2;
        self.pad(THREAD_KEY_MAX.saturating_sub(used));
    }
}

impl Default for Buf {
    fn default() -> Self {
        Self::new()
    }
}

/// `v` in decimal, right-aligned in a `NUM_WIDTH` array; the index of the first digit.
fn digits_of(mut v: u64) -> ([u8; NUM_WIDTH], usize) {
    let mut digits = [b'0'; NUM_WIDTH];
    let mut i = NUM_WIDTH;
    loop {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            return (digits, i);
        }
    }
}

pub fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b':' | b'/' | b'-')
}

fn sent_char(c: u8) -> u8 {
    if is_ident_char(c) {
        c
    } else {
        b'_'
    }
}

/// FNV-1a over the form of `s` that is sent (bad bytes replaced), at most `max` bytes.
pub fn ident_hash(s: &[u8], max: usize) -> u64 {
    s.iter()
        .take(max)
        .take_while(|&&c| c != 0)
        .fold(FNV_OFFSET, |h, &c| {
            (h ^ u64::from(sent_char(c))).wrapping_mul(FNV_PRIME)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(b: &Buf) -> String {
        String::from_utf8(b.as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn counters_are_right_aligned_in_twenty_characters() {
        let mut b = Buf::new();
        b.put_u64_w(5);
        assert_eq!(text(&b), format!("{}5", " ".repeat(19)));
        b.clear();
        b.put_u64_w(u64::MAX);
        assert_eq!(text(&b), "18446744073709551615");
        assert_eq!(b.len(), NUM_WIDTH);
    }

    #[test]
    fn unpadded_numbers_have_no_leading_spaces() {
        let mut b = Buf::new();
        b.put_u64(0);
        b.put_str(",");
        b.put_u64(1234);
        b.put_str(",");
        b.put_i64(-7);
        b.put_str(",");
        b.put_i64(i64::MIN);
        assert_eq!(text(&b), "0,1234,-7,-9223372036854775808");
    }

    #[test]
    fn signed_counters_keep_the_sign_inside_the_field() {
        let mut b = Buf::new();
        b.put_i64_w(-7);
        assert_eq!(text(&b), format!("{}-7", " ".repeat(18)));
        b.clear();
        b.put_i64_w(i64::MIN);
        assert_eq!(b.len(), NUM_WIDTH);
        assert!(text(&b).ends_with("-9223372036854775808"));
    }

    #[test]
    fn hex_is_sixteen_lowercase_digits() {
        let mut b = Buf::new();
        b.put_hex64(0xfb6c_f21a_75aa_d6ef);
        assert_eq!(text(&b), "fb6cf21a75aad6ef");
        b.clear();
        b.put_hex64(0xab);
        assert_eq!(text(&b), "00000000000000ab");
    }

    #[test]
    fn free_text_escapes_quotes_and_backslashes_and_masks_other_bytes() {
        let mut b = Buf::new();
        b.put_json_text("a\"b\\c\u{e9}\n".as_bytes(), MSG_MAX);
        assert_eq!(text(&b), "\"a\\\"b\\\\c???\"");
    }

    #[test]
    fn free_text_is_cut_at_the_limit() {
        let mut b = Buf::new();
        b.put_json_text(b"abcdef", 3);
        assert_eq!(text(&b), "\"abc\"");
    }

    #[test]
    fn ids_replace_bytes_outside_the_alphabet() {
        let mut b = Buf::new();
        b.put_ident(b"a b#c.d:e/f-g_h", ID_MAX);
        assert_eq!(text(&b), "\"a_b_c.d:e/f-g_h\"");
    }

    #[test]
    fn thread_keys_keep_the_hash_and_pad_to_sixty_four_bytes() {
        let mut b = Buf::new();
        b.put_thread_key_padded(b"t#1");
        assert_eq!(text(&b), format!("\"t#1\"{}", " ".repeat(61)));
    }

    #[test]
    fn a_full_buffer_sets_over_and_ignores_later_puts() {
        let mut b = Buf::new();
        b.pad(LINE_MAX);
        assert!(!b.is_over());
        b.put_str("x");
        assert!(b.is_over());
        assert_eq!(b.len(), LINE_MAX);
        b.put_str("y");
        assert_eq!(b.len(), LINE_MAX);
    }

    #[test]
    fn id_hash_uses_the_sent_form_and_the_limit() {
        assert_eq!(ident_hash(b"a b", ID_MAX), ident_hash(b"a_b", ID_MAX));
        assert_ne!(ident_hash(b"a", ID_MAX), ident_hash(b"ab", ID_MAX));
        assert_eq!(ident_hash(b"abcX", 3), ident_hash(b"abcY", 3));
        assert!(is_ident_char(b'/') && !is_ident_char(b'#') && !is_ident_char(b' '));
    }

    #[test]
    fn id_hash_matches_the_c_sdk_for_a_known_id() {
        // FNV-1a 64 of "basic.always.ok", computed independently of this code.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &c in b"basic.always.ok" {
            h = (h ^ u64::from(c)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        assert_eq!(ident_hash(b"basic.always.ok", ID_MAX), h);
    }
}
