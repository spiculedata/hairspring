//! Character rules shared by the event parser and by everything that re-checks guest strings after the fact (the
//! trial digest decoder, the run catalog). One definition, so the two can never disagree about what is acceptable.

/// Bytes allowed in ids and process tags.
pub fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-')
}

/// Thread keys additionally allow `#`, because unlabelled threads are named `t#<claim order>`.
pub fn is_thread_byte(b: u8) -> bool {
    is_ident_byte(b) || b == b'#'
}

/// Format (Cf) and line/paragraph separator (Zl, Zp) characters. The list follows the Unicode 15.1 general
/// categories (Cf, Zl, Zp); characters added to those categories in later versions are not covered. They can
/// reorder or hide text in a terminal or UI.
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// True for a non-empty identifier of at most `max` bytes: letters, digits and `. _ : / -`.
pub fn is_ident(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(is_ident_byte)
}

/// True for a non-empty thread key of at most `max` bytes: an identifier that may also contain `#`.
pub fn is_thread_key(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(is_thread_byte)
}

/// True for display text of at most `max` bytes with no control, format or line-separator characters.
pub fn is_plain_text(s: &str, max: usize) -> bool {
    s.len() <= max && !s.chars().any(|c| c.is_control() || is_invisible(c))
}
