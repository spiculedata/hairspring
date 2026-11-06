//! Rules for the list of recorded fields (`env.fields`): duplicates, entries with no value, size caps, and how the
//! informational mark travels.
use hairspring_core::bundle::Bundle;
use hairspring_core::environment::{
    compare, EnvironmentComparison, EnvironmentIdentity, FieldSpec, Grammar, OwnerFields, Registry,
    FIELD_LIST_PARAM,
};

fn identity(pairs: &[(&str, &str)]) -> EnvironmentIdentity {
    pairs
        .iter()
        .fold(EnvironmentIdentity::default(), |id, (k, v)| {
            id.with_field(*k, *v).unwrap()
        })
}

fn bundle_listing(pairs: &[(&str, &str)], list: &str) -> Bundle {
    identity(pairs)
        .apply_to(Bundle::new(1, "b", vec![]))
        .with_param(FIELD_LIST_PARAM, list)
}

fn is_malformed_list(list: &str) -> bool {
    let parsed =
        EnvironmentIdentity::from_bundle(&bundle_listing(&[("host.container", "none")], list));
    parsed.malformed == [FIELD_LIST_PARAM] && parsed.is_partly_unreadable()
}

#[test]
fn a_name_listed_twice_makes_the_list_malformed_in_either_order() {
    assert!(is_malformed_list("hermit.x,~hermit.x"));
    assert!(is_malformed_list("~hermit.x,hermit.x"));
    assert!(is_malformed_list("hermit.x,hermit.x"));
    assert!(is_malformed_list("~hermit.x,~hermit.x"));
    assert!(is_malformed_list("host.container,host.container"));
    assert!(is_malformed_list("host.container,~host.container"));
}

#[test]
fn distinct_names_are_not_duplicates() {
    assert!(!is_malformed_list("host.container,hermit.x,hermit.y"));
}

#[test]
fn a_known_field_listed_but_not_recorded_is_malformed_and_says_so() {
    let bundle = bundle_listing(&[("host.container", "none")], "env.base_env,host.container");
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert_eq!(parsed.malformed, ["env.base_env (listed, not recorded)"]);
    assert!(parsed.is_partly_unreadable());
    let line = parsed.warning().unwrap();
    assert!(
        line.contains("env.base_env (listed, not recorded)"),
        "{line}"
    );
}

#[test]
fn a_known_field_listed_and_recorded_is_fine() {
    let bundle = bundle_listing(&[("host.container", "none")], "host.container");
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert!(parsed.malformed.is_empty());
}

/// A list of exactly `len` characters made of distinct, valid names.
fn list_of_len(len: usize) -> (String, usize) {
    const ENTRY: usize = 12;
    let count = (len + 1) / (ENTRY + 1);
    let mut entries: Vec<String> = (0..count).map(|i| format!("hermit.n{i:04}")).collect();
    let pad = len + 1 - (ENTRY + 1) * count;
    let last = entries.last_mut().unwrap();
    last.push_str(&"z".repeat(pad));
    let text = entries.join(",");
    assert_eq!(text.len(), len);
    (text, count)
}

#[test]
fn the_list_may_be_4096_characters_and_no_more() {
    let (at_cap, count) = list_of_len(4096);
    let parsed = EnvironmentIdentity::from_bundle(&bundle_listing(&[], &at_cap));
    assert!(parsed.malformed.is_empty(), "{:?}", parsed.malformed);
    assert_eq!(parsed.unknown.len(), count);
    let (over, _) = list_of_len(4097);
    let parsed = EnvironmentIdentity::from_bundle(&bundle_listing(&[], &over));
    assert_eq!(parsed.malformed, [FIELD_LIST_PARAM]);
}

#[test]
fn a_listed_name_may_be_64_characters_and_no_more() {
    let at_cap = format!("hermit.{}", "a".repeat(64 - "hermit.".len()));
    assert_eq!(at_cap.len(), 64);
    let parsed = EnvironmentIdentity::from_bundle(&bundle_listing(&[], &at_cap));
    assert!(parsed.malformed.is_empty());
    assert_eq!(parsed.unknown_must_match(), [at_cap.as_str()]);
    let over = format!("{at_cap}a");
    let parsed = EnvironmentIdentity::from_bundle(&bundle_listing(&[], &over));
    assert_eq!(parsed.malformed, [FIELD_LIST_PARAM]);
}

const WRITER: [OwnerFields; 1] = [OwnerFields {
    owner: "writer",
    fields: &[
        FieldSpec::must_match("test.build", Grammar::Text),
        FieldSpec::informational("test.note", Grammar::Text),
    ],
}];
const OLDER_READER: [OwnerFields; 1] = [OwnerFields {
    owner: "reader",
    fields: &[FieldSpec::must_match("test.build", Grammar::Text)],
}];

#[test]
fn the_informational_mark_is_written_and_read_by_a_reader_that_lacks_the_field() {
    let writer = Registry::new(&WRITER);
    let recorded = EnvironmentIdentity::default()
        .with_field_in(&writer, "test.build", "1")
        .unwrap()
        .with_field_in(&writer, "test.note", "n")
        .unwrap();
    let bundle = recorded.apply_to_in(&writer, Bundle::new(1, "b", vec![]));
    assert_eq!(
        bundle.params.get(FIELD_LIST_PARAM).map(String::as_str),
        Some("test.build,~test.note")
    );
    let reader = Registry::new(&OLDER_READER);
    let parsed = EnvironmentIdentity::from_bundle_in(&reader, &bundle);
    assert_eq!(parsed.unknown.len(), 1);
    assert!(parsed.unknown_must_match().is_empty());
    assert!(!parsed.is_partly_unreadable());
}

#[test]
fn a_reader_applies_its_own_rule_to_a_known_field_whatever_the_bundle_marks() {
    let bundle = bundle_listing(&[("host.container", "none")], "~host.container");
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert!(parsed.malformed.is_empty());
    assert!(parsed.unknown.is_empty());
    let outcome = compare(
        parsed.identity.as_ref(),
        &identity(&[("host.container", "docker")]),
    );
    assert!(matches!(outcome, EnvironmentComparison::Differs(_)));
}
