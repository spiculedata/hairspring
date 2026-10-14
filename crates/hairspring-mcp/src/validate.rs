//! Strict validation of identifiers and arguments that arrive from untrusted callers.
use serde_json::Value;

pub const MAX_TARGET_LEN: usize = 64;

fn hex_tail(s: &str, prefix: &str, len: usize) -> bool {
    s.strip_prefix(prefix)
        .is_some_and(|t| t.len() == len && t.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub fn run_id(s: &str) -> bool {
    hex_tail(s, "run_", 16)
}

pub fn failure_id(s: &str) -> bool {
    hex_tail(s, "fail_", 16)
}

pub fn bundle_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn target_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_TARGET_LEN
}

pub type ArgResult<T> = Result<T, String>;

pub fn req_str<'a>(args: &'a Value, key: &str) -> ArgResult<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{key}` is required and must be a string"))
}

pub fn opt_str<'a>(args: &'a Value, key: &str) -> ArgResult<Option<&'a str>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

pub fn opt_u64(args: &Value, key: &str) -> ArgResult<Option<u64>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be a non-negative integer")),
    }
}

pub fn opt_bool(args: &Value, key: &str) -> ArgResult<Option<bool>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be a boolean")),
    }
}
