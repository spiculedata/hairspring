//! Canonical command values and the command hash.
//!
//! The canonical JSON has keys in byte order, no spaces, and every integer written as a decimal
//! string. In text, `"` and `\` are escaped and each control character below U+0020 is written
//! as `\u00xx` (lowercase hex, no short forms); everything else is written as it is.
use super::encoding::{normalize_sha256, sha256_hex};
use std::collections::BTreeMap;

/// Deepest nesting accepted in a command value.
pub const MAX_COMMAND_DEPTH: usize = 32;
/// First line of every command hash input.
pub const COMMAND_HASH_HEADER: &str = "hairspring-command v1\n";

/// A command as data: what a typed command carries, without any parser attached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandValue {
    Null,
    Bool(bool),
    /// Any integer from `i64::MIN` to `u64::MAX`; others are refused when written. Written as a
    /// decimal string.
    Int(i128),
    Text(String),
    List(Vec<CommandValue>),
    Object(BTreeMap<String, CommandValue>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    DuplicateKey(String),
    TooDeep,
    BadPointHash,
    /// An integer outside `i64::MIN..=u64::MAX`.
    IntOutOfRange,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::DuplicateKey(key) => write!(f, "command has a repeated key `{key}`"),
            CommandError::TooDeep => {
                write!(
                    f,
                    "command is nested more than {MAX_COMMAND_DEPTH} levels deep"
                )
            }
            CommandError::BadPointHash => f.write_str("point hash must be 64 lowercase hex digits"),
            CommandError::IntOutOfRange => {
                f.write_str("command integer is outside the i64 and u64 range")
            }
        }
    }
}

impl std::error::Error for CommandError {}

impl CommandValue {
    /// Builds an object from pairs in any order; a repeated key is an error.
    pub fn object(pairs: Vec<(String, CommandValue)>) -> Result<Self, CommandError> {
        let mut map = BTreeMap::new();
        for (key, value) in pairs {
            if map.contains_key(&key) {
                return Err(CommandError::DuplicateKey(key));
            }
            map.insert(key, value);
        }
        Ok(CommandValue::Object(map))
    }

    /// The canonical JSON text: keys in byte order, no spaces, integers as decimal strings.
    pub fn canonical_json(&self) -> Result<String, CommandError> {
        let mut out = String::new();
        write_value(self, 0, &mut out)?;
        Ok(out)
    }
}

fn write_value(value: &CommandValue, depth: usize, out: &mut String) -> Result<(), CommandError> {
    if depth > MAX_COMMAND_DEPTH {
        return Err(CommandError::TooDeep);
    }
    match value {
        CommandValue::Null => out.push_str("null"),
        CommandValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        CommandValue::Int(n) => {
            if !(i128::from(i64::MIN)..=i128::from(u64::MAX)).contains(n) {
                return Err(CommandError::IntOutOfRange);
            }
            write_text(&n.to_string(), out)
        }
        CommandValue::Text(s) => write_text(s, out),
        CommandValue::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, depth + 1, out)?;
            }
            out.push(']');
        }
        CommandValue::Object(map) => {
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_text(key, out);
                out.push(':');
                write_value(item, depth + 1, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_text(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `SHA-256("hairspring-command v1\n" + point_hash + "\n" + canonical JSON)`, as lowercase hex.
///
/// This is a cache key, not proof of reproducibility: a repeat of the same command at the same
/// point maps to the same hash.
pub fn command_hash(point_hash: &str, command: &CommandValue) -> Result<String, CommandError> {
    if normalize_sha256(point_hash).as_deref() != Some(point_hash) {
        return Err(CommandError::BadPointHash);
    }
    let json = command.canonical_json()?;
    let input = format!("{COMMAND_HASH_HEADER}{point_hash}\n{json}");
    Ok(sha256_hex(input.as_bytes()))
}
