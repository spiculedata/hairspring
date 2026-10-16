use hairspring_core::point::command::{CommandError, CommandValue, MAX_COMMAND_DEPTH};
use hairspring_core::point::command_hash;

const POINT: &str = "107baabb4434970617d69291598e3ee50e883b14a3d57bc77894fbd968e98d65";

fn text(s: &str) -> CommandValue {
    CommandValue::Text(s.to_string())
}

fn object(pairs: Vec<(&str, CommandValue)>) -> CommandValue {
    let owned = pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    CommandValue::object(owned).unwrap()
}

fn ints(values: &[i128]) -> CommandValue {
    CommandValue::List(values.iter().copied().map(CommandValue::Int).collect())
}

fn walk() -> CommandValue {
    object(vec![
        ("steps", CommandValue::Int(10)),
        ("seeds", ints(&[1, 2])),
    ])
}

#[test]
fn canonical_json_sorts_keys_drops_spaces_and_writes_integers_as_strings() {
    assert_eq!(
        walk().canonical_json().unwrap(),
        r#"{"seeds":["1","2"],"steps":"10"}"#
    );
    let nested = object(vec![
        (
            "b",
            object(vec![
                ("y", CommandValue::Null),
                ("x", CommandValue::Bool(true)),
            ]),
        ),
        ("a", CommandValue::List(vec![])),
        ("c", object(vec![])),
    ]);
    assert_eq!(
        nested.canonical_json().unwrap(),
        r#"{"a":[],"b":{"x":true,"y":null},"c":{}}"#
    );
}

#[test]
fn integers_cover_the_full_u64_and_i64_ranges() {
    let value = ints(&[0, -3, i64::MIN.into(), u64::MAX.into()]);
    assert_eq!(
        value.canonical_json().unwrap(),
        r#"["0","-3","-9223372036854775808","18446744073709551615"]"#
    );
}

#[test]
fn text_is_escaped_minimally_and_byte_exact() {
    let value = object(vec![("label", text("a\"b\\c\nd\u{1}\u{e9}/"))]);
    assert_eq!(
        value.canonical_json().unwrap(),
        "{\"label\":\"a\\\"b\\\\c\\u000ad\\u0001\u{e9}/\"}"
    );
    // Whitespace inside a string is data, not formatting.
    assert_ne!(
        text("a b").canonical_json().unwrap(),
        text("ab").canonical_json().unwrap()
    );
    assert_ne!(
        text("A").canonical_json().unwrap(),
        text("a").canonical_json().unwrap()
    );
}

#[test]
fn key_order_and_insertion_order_do_not_change_the_hash() {
    let forward = object(vec![
        ("steps", CommandValue::Int(10)),
        ("seeds", ints(&[1, 2])),
        ("meta", object(vec![("a", text("x")), ("b", text("y"))])),
    ]);
    let backward = object(vec![
        ("meta", object(vec![("b", text("y")), ("a", text("x"))])),
        ("seeds", ints(&[1, 2])),
        ("steps", CommandValue::Int(10)),
    ]);
    assert_eq!(
        command_hash(POINT, &forward),
        command_hash(POINT, &backward)
    );
}

#[test]
fn list_order_is_part_of_the_command() {
    let a = object(vec![("seeds", ints(&[1, 2]))]);
    let b = object(vec![("seeds", ints(&[2, 1]))]);
    assert_ne!(
        command_hash(POINT, &a).unwrap(),
        command_hash(POINT, &b).unwrap()
    );
}

#[test]
fn golden_command_hashes() {
    let action = object(vec![("action", text("0000000000000001"))]);
    assert_eq!(
        command_hash(POINT, &action).unwrap(),
        "0f6328371f31f17a178ab638a3cec1912e53ded1caa0c16741e2c313be6826ee"
    );
    assert_eq!(
        command_hash(POINT, &walk()).unwrap(),
        "ce9d67c898ee722398b61c6abe69c4a9469e7ce7a1e59e4fe88715d348021a32"
    );
    let label = object(vec![("label", text("a\"b\\c\nd\u{1}\u{e9}/"))]);
    assert_eq!(
        command_hash(POINT, &label).unwrap(),
        "82da1c1a03c7cfa18466a52c9555dd6bfab16ca9445797ea50254642c53a5271"
    );
}

#[test]
fn changing_the_point_or_one_command_field_changes_the_hash() {
    let base = command_hash(POINT, &walk()).unwrap();
    let other_point = "0".repeat(64);
    let more_steps = object(vec![
        ("steps", CommandValue::Int(11)),
        ("seeds", ints(&[1, 2])),
    ]);
    let other_seed = object(vec![
        ("steps", CommandValue::Int(10)),
        ("seeds", ints(&[1, 3])),
    ]);
    let extra_key = object(vec![
        ("steps", CommandValue::Int(10)),
        ("seeds", ints(&[1, 2])),
        ("verify", CommandValue::Bool(false)),
    ]);
    let variants = [
        command_hash(&other_point, &walk()).unwrap(),
        command_hash(POINT, &more_steps).unwrap(),
        command_hash(POINT, &other_seed).unwrap(),
        command_hash(POINT, &extra_key).unwrap(),
    ];
    for v in &variants {
        assert_ne!(v, &base);
    }
    let unique: std::collections::HashSet<_> = variants.iter().collect();
    assert_eq!(unique.len(), variants.len());
}

#[test]
fn duplicate_keys_are_rejected() {
    let pairs = vec![
        ("a".to_string(), CommandValue::Null),
        ("a".to_string(), CommandValue::Null),
    ];
    assert_eq!(
        CommandValue::object(pairs),
        Err(CommandError::DuplicateKey("a".to_string()))
    );
}

#[test]
fn nesting_is_bounded() {
    let mut value = CommandValue::Null;
    for _ in 0..MAX_COMMAND_DEPTH {
        value = CommandValue::List(vec![value]);
    }
    assert!(value.canonical_json().is_ok());
    let too_deep = CommandValue::List(vec![value]);
    assert_eq!(too_deep.canonical_json(), Err(CommandError::TooDeep));
    assert_eq!(command_hash(POINT, &too_deep), Err(CommandError::TooDeep));
}

#[test]
fn the_point_hash_must_be_a_lowercase_sha256() {
    let bad = [
        "",
        "abc",
        &POINT.to_uppercase(),
        &format!("{POINT}0"),
        &"g".repeat(64),
    ];
    for point in bad {
        assert_eq!(
            command_hash(point, &walk()),
            Err(CommandError::BadPointHash),
            "{point:?}"
        );
    }
}

#[test]
fn errors_have_readable_messages() {
    assert_eq!(
        CommandError::DuplicateKey("a".to_string()).to_string(),
        "command has a repeated key `a`"
    );
    assert_eq!(
        CommandError::TooDeep.to_string(),
        "command is nested more than 32 levels deep"
    );
    assert_eq!(
        CommandError::BadPointHash.to_string(),
        "point hash must be 64 lowercase hex digits"
    );
}
