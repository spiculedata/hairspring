//! Text encoding shared by the identity text and the hashes.
//!
//! Values are percent-escaped as in bundle format v2 (`%`, LF and CR), and also `,` inside list
//! items, so a value can never start a new line or split a list item.

/// A required input that may not have been recorded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Recorded<T> {
    /// The input was not recorded; the run has no identity.
    #[default]
    Missing,
    Value(T),
}

impl<T> Recorded<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Recorded::Missing => None,
            Recorded::Value(v) => Some(v),
        }
    }
}

/// Percent-escapes a scalar value: `%`, LF and CR.
pub fn escape_value(text: &str) -> String {
    escape(text, false)
}

fn escape_item(text: &str) -> String {
    escape(text, true)
}

fn escape(text: &str, escape_comma: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            ',' if escape_comma => out.push_str("%2C"),
            other => out.push(other),
        }
    }
    out
}

/// Encodes a list as `<count>:<item>,<item>`, with items escaped. The count keeps an empty list
/// (`0:`) distinct from a list holding one empty item (`1:`).
pub fn encode_list(items: &[String]) -> String {
    let body: Vec<String> = items.iter().map(|i| escape_item(i)).collect();
    format!("{}:{}", items.len(), body.join(","))
}

/// Encodes an optional string as `none` or `some:<escaped>`.
pub fn encode_optional_text(value: Option<&str>) -> String {
    match value {
        None => "none".to_string(),
        Some(v) => format!("some:{}", escape_value(v)),
    }
}

/// Encodes an optional number as `none` or `some:<decimal>`.
pub fn encode_optional_number(value: Option<u64>) -> String {
    match value {
        None => "none".to_string(),
        Some(v) => format!("some:{v}"),
    }
}

pub fn encode_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

/// SHA-256 of the bytes, as lowercase hex.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

const SHA256_HEX_LEN: usize = 64;

/// Returns the lowercase form of a SHA-256 hex digest, or `None` if the text is not one.
pub fn normalize_sha256(text: &str) -> Option<String> {
    let is_digest = text.len() == SHA256_HEX_LEN && text.bytes().all(|b| b.is_ascii_hexdigit());
    is_digest.then(|| text.to_ascii_lowercase())
}
