//! The environment field registry and the list of recorded fields a bundle carries.
use hairspring_core::bundle::Bundle;
use hairspring_core::environment::{
    compare, compare_in, known_field_names, EnvironmentComparison, EnvironmentIdentity, FieldSpec,
    Grammar, OwnerFields, Registry, Rule, ENGINE_FIELDS, FIELD_LIST_PARAM, HERMIT_FIELDS,
    MAX_LISTED_CHANGES, REGISTRY,
};
use std::collections::BTreeSet;

fn identity(pairs: &[(&str, &str)]) -> EnvironmentIdentity {
    pairs
        .iter()
        .fold(EnvironmentIdentity::default(), |id, (k, v)| {
            id.with_field(*k, *v).unwrap()
        })
}

fn bundle_with(pairs: &[(&str, &str)]) -> Bundle {
    identity(pairs).apply_to(Bundle::new(1, "b", vec![]))
}

fn names(owner: &OwnerFields) -> BTreeSet<&'static str> {
    owner.fields.iter().map(|f| f.name).collect()
}

#[test]
fn each_name_is_declared_once_and_by_the_owner_of_its_namespace() {
    let engine = names(&ENGINE_FIELDS);
    let hermit = names(&HERMIT_FIELDS);
    assert!(engine.is_disjoint(&hermit));
    let all: BTreeSet<&str> = known_field_names().collect();
    assert_eq!(all.len(), 18);
    assert_eq!(all, engine.union(&hermit).copied().collect());
    assert!(hermit
        .iter()
        .all(|n| n.starts_with("hermit.") || n.starts_with("host.loader.")));
    assert!(engine
        .iter()
        .all(|n| n.starts_with("host.") || n.starts_with("env.") || n.starts_with("target.")));
    assert!(engine.iter().all(|n| !n.starts_with("host.loader.")));
}

#[test]
fn every_existing_field_is_must_match() {
    assert!(REGISTRY.fields().all(|f| f.rule == Rule::MustMatch));
}

#[test]
fn only_the_guest_passed_field_has_a_default_for_old_bundles() {
    let defaults: Vec<_> = REGISTRY
        .fields()
        .filter_map(|f| f.default_when_absent.map(|d| (f.name, d)))
        .collect();
    assert_eq!(defaults, [("env.guest.passed", "none")]);
}

const OWNERS: [OwnerFields; 1] = [OwnerFields {
    owner: "test",
    fields: &[
        FieldSpec::must_match("test.build", Grammar::Text),
        FieldSpec::informational("test.note", Grammar::Text),
    ],
}];
const TEST_REGISTRY: Registry = Registry::new(&OWNERS);

fn test_identity(build: &str, note: &str) -> EnvironmentIdentity {
    EnvironmentIdentity::default()
        .with_field_in(&TEST_REGISTRY, "test.build", build)
        .unwrap()
        .with_field_in(&TEST_REGISTRY, "test.note", note)
        .unwrap()
}

#[test]
fn an_informational_field_never_makes_the_environment_differ() {
    let outcome = compare_in(
        &TEST_REGISTRY,
        Some(&test_identity("1", "x")),
        &test_identity("1", "y"),
    );
    assert_eq!(outcome, EnvironmentComparison::Same);
}

#[test]
fn a_must_match_field_in_the_same_registry_still_differs() {
    let outcome = compare_in(
        &TEST_REGISTRY,
        Some(&test_identity("1", "x")),
        &test_identity("2", "x"),
    );
    assert!(outcome.differs());
}

#[test]
fn a_registry_that_does_not_know_a_name_refuses_it() {
    assert!(EnvironmentIdentity::default()
        .with_field_in(&TEST_REGISTRY, "hermit.version", "1")
        .is_err());
    assert!(EnvironmentIdentity::default()
        .with_field("test.build", "1")
        .is_err());
}

#[test]
fn a_new_bundle_lists_the_fields_it_recorded_in_name_order() {
    let bundle = bundle_with(&[("target.cwd.sha256", "ab"), ("host.container", "none")]);
    assert_eq!(
        bundle.params.get(FIELD_LIST_PARAM).map(String::as_str),
        Some("host.container,target.cwd.sha256")
    );
    assert!(bundle.encode().starts_with("hairspring-bundle v3\n"));
}

#[test]
fn a_bundle_with_nothing_recorded_has_no_list_and_stays_v2() {
    let bundle = identity(&[]).apply_to(Bundle::new(1, "b", vec![]));
    assert!(!bundle.params.contains_key(FIELD_LIST_PARAM));
    assert!(bundle.encode().starts_with("hairspring-bundle v2\n"));
}

#[test]
fn the_list_survives_a_round_trip_and_is_not_a_compared_field() {
    let bundle = bundle_with(&[("host.container", "none"), ("env.base_env", "default")]);
    let decoded = Bundle::decode(&bundle.encode()).unwrap();
    assert_eq!(decoded, bundle);
    let parsed = EnvironmentIdentity::from_bundle(&decoded);
    let identity = parsed.identity.unwrap();
    assert!(identity.get(FIELD_LIST_PARAM).is_none());
    assert_eq!(identity.fields().count(), 2);
    assert!(parsed.malformed.is_empty());
    assert!(parsed.unknown.is_empty());
}

#[test]
fn a_bundle_from_before_the_list_existed_has_no_unknown_fields() {
    let mut bundle = bundle_with(&[("host.container", "none")]);
    bundle.params.remove(FIELD_LIST_PARAM);
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert!(parsed.unknown.is_empty());
    assert!(!parsed.is_partly_unreadable());
    assert_eq!(parsed.unknown_warning(), None);
}

#[test]
fn a_listed_name_this_build_does_not_know_is_an_unknown_must_match_field() {
    let bundle = bundle_with(&[("host.container", "none")])
        .with_param(FIELD_LIST_PARAM, "host.container,hermit.future_flag");
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert_eq!(parsed.unknown_must_match(), ["hermit.future_flag"]);
    assert!(parsed.is_partly_unreadable());
    let line = parsed.unknown_warning().unwrap();
    assert!(line.starts_with("environment differs: "), "{line}");
    assert!(line.contains("hermit.future_flag"), "{line}");
    assert!(line.contains("upgrade Hairspring"), "{line}");
}

#[test]
fn an_unknown_name_marked_informational_is_read_but_never_makes_a_miss_inconclusive() {
    let bundle = bundle_with(&[("host.container", "none")])
        .with_param(FIELD_LIST_PARAM, "host.container,~hermit.future_note")
        .with_param("hermit.future_note", "whatever");
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert_eq!(parsed.unknown.len(), 1);
    assert!(parsed.unknown_must_match().is_empty());
    assert!(!parsed.is_partly_unreadable());
    assert_eq!(parsed.unknown_warning(), None);
}

#[test]
fn an_unknown_param_in_an_owned_namespace_counts_even_without_a_list() {
    let bundle = Bundle::new(1, "b", vec![]).with_param("host.future_fact", "x");
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert_eq!(parsed.unknown_must_match(), ["host.future_fact"]);
    assert!(parsed.identity.is_none());
}

#[test]
fn a_param_outside_every_owned_namespace_is_an_ordinary_replay_parameter() {
    let bundle = Bundle::new(1, "b", vec![])
        .with_param("sched_seed", "3")
        .with_param("hermit_args", "--chaos")
        .with_param("qemu.version", "9")
        .with_param("events_transport", "stream");
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert!(parsed.unknown.is_empty());
    assert!(!parsed.is_partly_unreadable());
}

#[test]
fn an_explicit_list_entry_wins_over_the_namespace_default() {
    let bundle = Bundle::new(1, "b", vec![])
        .with_param(FIELD_LIST_PARAM, "~host.future_note")
        .with_param("host.future_note", "x");
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert!(parsed.unknown_must_match().is_empty());
    assert_eq!(parsed.unknown.len(), 1);
}

#[test]
fn a_list_that_cannot_be_read_is_malformed_not_guessed_at() {
    for bad in [
        "host.container,,env.base_env",
        "Host.Container",
        "host.container,\u{202e}evil",
        "a b",
        &"x".repeat(5000),
    ] {
        let bundle = bundle_with(&[("host.container", "none")]).with_param(FIELD_LIST_PARAM, bad);
        let parsed = EnvironmentIdentity::from_bundle(&bundle);
        assert_eq!(parsed.malformed, [FIELD_LIST_PARAM], "{bad:?}");
        assert!(parsed.is_partly_unreadable(), "{bad:?}");
    }
}

#[test]
fn an_unknown_name_that_cannot_be_shown_safely_is_reported_without_its_text() {
    let bundle = Bundle::new(1, "b", vec![]).with_param("host.\u{202e}evil\nline", "x");
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert_eq!(parsed.malformed, ["(unreadable field name)"]);
    assert!(parsed.unknown.is_empty());
    let shown = parsed.warning().unwrap();
    assert!(
        !shown.contains('\u{202e}') && !shown.contains('\n'),
        "{shown:?}"
    );
}

#[test]
fn many_unknown_fields_are_listed_up_to_the_limit_and_counted() {
    let many: Vec<String> = (0..50).map(|i| format!("hermit.future_{i:02}")).collect();
    let bundle = Bundle::new(1, "b", vec![]).with_param(FIELD_LIST_PARAM, many.join(","));
    let line = EnvironmentIdentity::from_bundle(&bundle)
        .unknown_warning()
        .unwrap();
    assert!(
        line.contains(&format!("and {} more", 50 - MAX_LISTED_CHANGES)),
        "{line}"
    );
    assert!(line.len() < 1000, "{}", line.len());
}

#[test]
fn comparison_of_known_fields_is_unaffected_by_unknown_ones() {
    let bundle = bundle_with(&[("host.container", "none")])
        .with_param(FIELD_LIST_PARAM, "host.container,hermit.future_flag");
    let parsed = EnvironmentIdentity::from_bundle(&bundle);
    assert_eq!(
        compare(
            parsed.identity.as_ref(),
            &identity(&[("host.container", "none")])
        ),
        EnvironmentComparison::Same
    );
}

/// A small deterministic generator, so the fuzz below needs no dependency and fails the same way every time.
fn next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

#[test]
fn arbitrary_params_never_panic_and_never_put_unsafe_text_in_a_warning() {
    const ALPHABET: &[char] = &[
        'a', 'z', '0', '.', '_', '-', ',', '~', ' ', '\n', '\u{202e}', 'A', '=', '%', 'h', 'o',
    ];
    let mut state = 7;
    for _ in 0..3000 {
        let len = (next(&mut state) % 40) as usize;
        let text = |state: &mut u64| -> String {
            (0..len)
                .map(|_| ALPHABET[(next(state) % ALPHABET.len() as u64) as usize])
                .collect()
        };
        let (name, value) = (text(&mut state), text(&mut state));
        let namespaced = format!("host.{name}");
        let bundle = Bundle::new(1, "b", vec![])
            .with_param(FIELD_LIST_PARAM, value.clone())
            .with_param(namespaced, value);
        let parsed = EnvironmentIdentity::from_bundle(&bundle);
        for line in parsed.warning().into_iter().chain(parsed.unknown_warning()) {
            assert!(
                !line
                    .chars()
                    .any(hairspring_core::environment::is_unsafe_display_char),
                "{line:?}"
            );
        }
        assert!(parsed.unknown.iter().all(|f| f.name.len() <= 64));
    }
}
