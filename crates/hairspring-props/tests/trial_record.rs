//! Kept trial detail: a versioned JSON record of one trial's folded events, bounded in size and re-checked on receipt.
mod common;

use common::*;
use hairspring_props::record::{TrialRecord, MAX_DETAIL_BYTES};
use hairspring_props::*;

fn props() -> TrialProps {
    fold(&single(AssertKind::Always, 3, 1))
}

#[test]
fn a_record_round_trips_through_bytes() {
    let original = props();
    let bytes = TrialRecord::encode(&original).expect("fits");
    let back = TrialRecord::decode(&bytes).expect("valid");
    assert_eq!(back.props, original);
    assert!(!back.truncated);
    assert!(bytes.len() <= MAX_DETAIL_BYTES);
}

#[test]
fn a_record_names_its_version() {
    let bytes = TrialRecord::encode(&props()).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("{\"v\":1"), "{text}");
}

#[test]
fn an_unknown_version_or_unknown_fields_are_refused() {
    let text = String::from_utf8(TrialRecord::encode(&props()).unwrap()).unwrap();
    let newer = text.replacen("\"v\":1", "\"v\":2", 1);
    assert!(TrialRecord::decode(newer.as_bytes()).is_err());
    let extra = text.replacen("{\"v\":1", "{\"v\":1,\"surprise\":true", 1);
    assert!(TrialRecord::decode(extra.as_bytes()).is_err());
    assert!(TrialRecord::decode(b"not json").is_err());
    assert!(TrialRecord::decode(b"").is_err());
}

#[test]
fn an_oversized_record_drops_its_property_list_and_says_so() {
    let mut big = props();
    let template = big.properties[0].clone();
    big.properties = (0..3000)
        .map(|i| TrialProperty {
            id: format!("module.component.property_{i:05}"),
            msg: "m".repeat(512),
            ..template.clone()
        })
        .collect();
    let bytes = TrialRecord::encode(&big).expect("the fallback fits");
    assert!(bytes.len() <= MAX_DETAIL_BYTES);
    let back = TrialRecord::decode(&bytes).unwrap();
    assert!(back.truncated);
    assert!(back.props.properties.is_empty());
    assert_eq!(back.props.procs, big.procs);
}

#[test]
fn hostile_strings_in_a_record_are_refused() {
    let base = TrialRecord::encode(&props()).unwrap();
    let text = String::from_utf8(base).unwrap();
    for (from, to) in [
        ("\"x.id\"", "\"x id\""),
        ("\"x.id\"", "\"bidi\\u202e\""),
        ("\"m x.id\"", "\"line\\nbreak\""),
        ("\"p1\"", "\"p 1\""),
        ("\"w-0\"", "\"w\\u0000\""),
    ] {
        let hostile = text.replace(from, to);
        assert_ne!(hostile, text, "the fixture must contain {from}");
        assert!(TrialRecord::decode(hostile.as_bytes()).is_err(), "{to}");
    }
}

#[test]
fn counts_above_the_protocol_limits_are_refused() {
    let mut many = props();
    let proc = many.procs[0].clone();
    many.procs = (0..65).map(|_| proc.clone()).collect();
    let bytes = serde_json::to_vec(&serde_json::json!({"v": 1, "truncated": false, "props": many}))
        .unwrap();
    assert!(TrialRecord::decode(&bytes).is_err());
}

#[test]
fn input_over_the_size_cap_is_refused_before_parsing() {
    let big = vec![b' '; MAX_DETAIL_BYTES + 1];
    assert!(TrialRecord::decode(&big).is_err());
}
