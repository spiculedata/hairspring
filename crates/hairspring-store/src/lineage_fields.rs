//! Content checks for the tree edge and command fields, shared by both stores. The stores already bound these fields
//! by length and NUL; these checks add what the values must look like.
//!
//! * `edge_ref`, `point_hash` and `command_hash` are SHA-256 digests in lower-case hexadecimal, the encoding of
//!   [`crate::sha256_hex`] and of the blob store's keys.
//! * `command_json` is one well-formed JSON document of at most [`MAX_COMMAND_JSON_DEPTH`] levels of nesting. Its size
//!   is bounded by [`crate::MAX_COMMAND_JSON_BYTES`] before this module sees it.
use crate::lineage::MAX_COMMAND_JSON_DEPTH;

const SHA256_HEX_LEN: usize = 64;

/// Whether `s` is a SHA-256 digest in lower-case hexadecimal.
pub(crate) fn is_sha256_hex(s: &str) -> bool {
    s.len() == SHA256_HEX_LEN && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether `text` is one JSON document nested no deeper than [`MAX_COMMAND_JSON_DEPTH`].
pub(crate) fn is_bounded_json(text: &str) -> bool {
    // serde_json refuses documents nested past its own recursion limit, so this cannot overflow the stack.
    serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok()
        && nesting_depth(text) <= MAX_COMMAND_JSON_DEPTH
}

/// The deepest nesting of `[` and `{` in text already known to be valid JSON; brackets inside strings do not count.
fn nesting_depth(valid_json: &str) -> usize {
    let (mut depth, mut deepest) = (0usize, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for b in valid_json.bytes() {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lower_case_sha256_hex_is_a_digest() {
        let ok = "0123456789abcdef".repeat(4);
        assert!(is_sha256_hex(&ok));
        assert!(!is_sha256_hex(&ok[1..]));
        assert!(!is_sha256_hex(&format!("{ok}0")));
        assert!(!is_sha256_hex(&ok.to_uppercase()));
        assert!(!is_sha256_hex(&format!("{}g", &ok[1..])));
        assert!(!is_sha256_hex(""));
    }

    #[test]
    fn nesting_counts_brackets_outside_strings_only() {
        assert_eq!(nesting_depth("1"), 0);
        assert_eq!(nesting_depth(r#"{"a":[1,{"b":2}]}"#), 3);
        assert_eq!(nesting_depth(r#"["[[[[", "\"[[", "\\"]"#), 1);
        assert_eq!(nesting_depth(r#"["\\", ["x"]]"#), 2);
    }

    #[test]
    fn json_must_be_well_formed_and_not_too_deep() {
        let nested = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(is_bounded_json(r#"{"verb":"step","n":[1,2,3]}"#));
        assert!(is_bounded_json("null"));
        assert!(is_bounded_json(&nested(MAX_COMMAND_JSON_DEPTH)));
        assert!(!is_bounded_json(&nested(MAX_COMMAND_JSON_DEPTH + 1)));
        assert!(!is_bounded_json(&nested(100_000)));
        assert!(!is_bounded_json(""));
        assert!(!is_bounded_json("{"));
        assert!(!is_bounded_json("{} {}"));
    }
}
