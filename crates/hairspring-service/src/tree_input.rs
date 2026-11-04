//! Checks on the tree edge and command fields of a request from a worker, made before the store is touched.
//!
//! A worker is untrusted: every value here is refused unless it has the exact form the store keeps. The stores repeat
//! the form checks as a second line of defence; these give the caller a typed reason naming the field.
//!
//! | Field | Accepted |
//! |---|---|
//! | `edge_ref`, `point_hash`, `command_hash` | 64 characters of lower-case hexadecimal (a SHA-256 digest) |
//! | `command_json` | one JSON object, 1 to [`MAX_COMMAND_JSON_BYTES`] bytes, no NUL, at most [`MAX_COMMAND_JSON_DEPTH`] levels of nesting |
//!
//! Whether `command_hash` equals the hash of a canonical form of `command_json` is not checked: it is a cache key
//! the worker supplies, and which canonical form applies is not settled.
use crate::ServiceError;
use hairspring_store::{
    is_sha256_hex, nesting_depth, NewBranch, NewNode, MAX_COMMAND_JSON_BYTES,
    MAX_COMMAND_JSON_DEPTH,
};
use std::fmt;

/// Why a tree field was refused. Each variant names the field, so the message tells the worker what to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeFieldError {
    /// The named field is not a SHA-256 digest in lower-case hexadecimal.
    BadDigest(&'static str),
    CommandEmpty,
    CommandTooLong,
    CommandHasNul,
    CommandTooDeep,
    CommandNotJson,
    /// The command is JSON, but not an object.
    CommandNotObject,
}

impl fmt::Display for TreeFieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadDigest(field) => write!(f, "{field} must be 64 lower-case hexadecimal digits"),
            Self::CommandEmpty => f.write_str("command_json is empty"),
            Self::CommandTooLong => {
                write!(
                    f,
                    "command_json is longer than {MAX_COMMAND_JSON_BYTES} bytes"
                )
            }
            Self::CommandHasNul => f.write_str("command_json contains a NUL character"),
            Self::CommandTooDeep => {
                write!(
                    f,
                    "command_json is nested more than {MAX_COMMAND_JSON_DEPTH} levels deep"
                )
            }
            Self::CommandNotJson => f.write_str("command_json is not well-formed JSON"),
            Self::CommandNotObject => f.write_str("command_json must be a JSON object"),
        }
    }
}

impl std::error::Error for TreeFieldError {}

impl From<TreeFieldError> for ServiceError {
    fn from(e: TreeFieldError) -> Self {
        ServiceError::InvalidRequest(e.to_string())
    }
}

fn check_digest(field: &'static str, value: &str) -> Result<(), TreeFieldError> {
    if is_sha256_hex(value) {
        Ok(())
    } else {
        Err(TreeFieldError::BadDigest(field))
    }
}

/// Checks the edge reference of a node offered by a worker; no edge reference is allowed.
pub fn check_edge_ref(edge_ref: Option<&str>) -> Result<(), TreeFieldError> {
    edge_ref.map_or(Ok(()), |e| check_digest("edge_ref", e))
}

/// Checks a command as the worker sent it: size, NUL, nesting, well-formedness and shape, in that order.
pub fn check_command_json(text: &str) -> Result<(), TreeFieldError> {
    if text.is_empty() {
        return Err(TreeFieldError::CommandEmpty);
    }
    if text.len() > MAX_COMMAND_JSON_BYTES {
        return Err(TreeFieldError::CommandTooLong);
    }
    if text.contains('\0') {
        return Err(TreeFieldError::CommandHasNul);
    }
    // The depth is measured by a loop that does not recurse, so the parser is only given text within the bound.
    if nesting_depth(text) > MAX_COMMAND_JSON_DEPTH {
        return Err(TreeFieldError::CommandTooDeep);
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(_)) => Ok(()),
        Ok(_) => Err(TreeFieldError::CommandNotObject),
        Err(_) => Err(TreeFieldError::CommandNotJson),
    }
}

/// Checks the edge and command fields of a node.
pub fn check_new_node(node: &NewNode) -> Result<(), TreeFieldError> {
    check_edge_ref(node.edge_ref.as_deref())
}

/// Checks the hash and command fields of a branch.
pub fn check_new_branch(branch: &NewBranch) -> Result<(), TreeFieldError> {
    check_digest("point_hash", &branch.point_hash)?;
    check_digest("command_hash", &branch.command_hash)?;
    check_command_json(&branch.command_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_store::{CutKind, NodeOrigin, Operator, SeedSource};

    fn digest() -> String {
        "0123456789abcdef".repeat(4)
    }

    fn node(edge_ref: Option<String>) -> NewNode {
        NewNode {
            run_id: "r".into(),
            seq: 1,
            parent_node: Some(0),
            operator: Operator::Step,
            cut_kind: CutKind::Step,
            cut_at: 1,
            suffix_seed: None,
            edge_ref,
            origin: NodeOrigin::Exploration,
            reporter_id: "w".into(),
            reporter_trusted: false,
        }
    }

    fn branch(point: &str, command: &str, json: &str) -> NewBranch {
        NewBranch {
            run_id: "b".into(),
            tree_id: "t".into(),
            source_node: 0,
            cut_kind: CutKind::Step,
            cut_at: 1,
            point_hash: point.into(),
            command_json: json.into(),
            command_hash: command.into(),
            seed_source: SeedSource::Caller,
            created_by: "k".into(),
        }
    }

    /// A small deterministic generator, so the property-style tests need no extra crate and never flake.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    #[test]
    fn edge_ref_is_absent_or_a_digest() {
        assert_eq!(check_new_node(&node(None)), Ok(()));
        assert_eq!(check_new_node(&node(Some(digest()))), Ok(()));
        for bad in [
            String::new(),
            digest().to_uppercase(),
            digest()[1..].to_string(),
            format!("{}0", digest()),
            format!("{}g", &digest()[1..]),
            format!("{}\0", &digest()[1..]),
            "é".repeat(32),
        ] {
            assert_eq!(
                check_new_node(&node(Some(bad.clone()))),
                Err(TreeFieldError::BadDigest("edge_ref")),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn branch_hashes_name_the_field_that_is_wrong() {
        let ok = r#"{"verb":"step"}"#;
        assert_eq!(check_new_branch(&branch(&digest(), &digest(), ok)), Ok(()));
        assert_eq!(
            check_new_branch(&branch("short", &digest(), ok)),
            Err(TreeFieldError::BadDigest("point_hash"))
        );
        assert_eq!(
            check_new_branch(&branch(&digest(), &digest().to_uppercase(), ok)),
            Err(TreeFieldError::BadDigest("command_hash"))
        );
    }

    #[test]
    fn command_json_refusals_are_typed() {
        let nested = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        let deep_ok = format!(r#"{{"a":{}}}"#, nested(MAX_COMMAND_JSON_DEPTH - 1));
        let deep_bad = format!(r#"{{"a":{}}}"#, nested(MAX_COMMAND_JSON_DEPTH));
        let long = format!(r#"{{"a":"{}"}}"#, "x".repeat(MAX_COMMAND_JSON_BYTES));
        let cases: Vec<(String, Result<(), TreeFieldError>)> = vec![
            (r#"{"n":["1"]}"#.into(), Ok(())),
            (deep_ok, Ok(())),
            (deep_bad, Err(TreeFieldError::CommandTooDeep)),
            (nested(10_000), Err(TreeFieldError::CommandTooDeep)),
            (String::new(), Err(TreeFieldError::CommandEmpty)),
            (long, Err(TreeFieldError::CommandTooLong)),
            ("{\"a\":\"\0\"}".into(), Err(TreeFieldError::CommandHasNul)),
            ("{".into(), Err(TreeFieldError::CommandNotJson)),
            ("{} {}".into(), Err(TreeFieldError::CommandNotJson)),
            (
                r#"{"v":"\ud800"}"#.into(),
                Err(TreeFieldError::CommandNotJson),
            ),
            ("[1]".into(), Err(TreeFieldError::CommandNotObject)),
            ("null".into(), Err(TreeFieldError::CommandNotObject)),
            ("\"s\"".into(), Err(TreeFieldError::CommandNotObject)),
        ];
        for (text, want) in cases {
            let shown: String = text.chars().take(40).collect();
            assert_eq!(check_command_json(&text), want, "{shown:?}");
        }
    }

    #[test]
    fn arbitrary_text_never_panics_and_only_objects_pass() {
        let alphabet: Vec<char> = "{}[]\",:\\ \0\u{7f}0129atrue\u{d7ff}é😀".chars().collect();
        let mut rng = Lcg(7);
        for _ in 0..5_000 {
            let len = (rng.next() % 200) as usize;
            let text: String = (0..len)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            if check_command_json(&text).is_ok() {
                assert!(matches!(
                    serde_json::from_str::<serde_json::Value>(&text),
                    Ok(serde_json::Value::Object(_))
                ));
            }
            let edge_ok = check_edge_ref(Some(&text)).is_ok();
            assert_eq!(edge_ok, is_sha256_hex(&text));
        }
        assert_eq!(check_command_json("{}"), Ok(()));
    }

    #[test]
    fn oversized_and_deep_values_are_refused_without_recursion() {
        let deep = format!("{}x", "{\"a\":[".repeat(MAX_COMMAND_JSON_BYTES / 6));
        assert_eq!(
            check_command_json(&deep),
            Err(TreeFieldError::CommandTooDeep)
        );
        let huge = "9".repeat(MAX_COMMAND_JSON_BYTES * 4);
        assert_eq!(
            check_command_json(&huge),
            Err(TreeFieldError::CommandTooLong)
        );
        assert!(check_new_node(&node(Some("a".repeat(1 << 20)))).is_err());
    }

    #[test]
    fn a_field_error_becomes_an_invalid_request_naming_the_field() {
        let e: ServiceError = TreeFieldError::BadDigest("edge_ref").into();
        assert!(matches!(&e, ServiceError::InvalidRequest(m) if m.contains("edge_ref")));
    }
}
